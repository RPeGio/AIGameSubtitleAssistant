// ─── MOSS（ggml）后端探测与判定 ───────────────────────────
// 实现设计文档 docs/engines_vulkan_design.md §3.2（三级探测）/ §3.4（设备矩阵）：
//   L0 机器能力：nvidia-smi / vulkaninfo 探机器"能跑什么"
//   L1 资产一致性：.backend 标记 + ggml-{cuda,vulkan}.dll 名，探"装了什么"
//   L2 实证：`info <model>` 的 stderr `[mt I] backend:` 行，唯一可信判据
// 语义：显式请求（cpu/cuda/vulkan）不可用 = 硬报错，绝不自动降级
//（ggml 静默降级时 exit code 仍为 0，L2 行才是真实后端）；只有 auto（空）
// 允许 ggml 自选最优。设备名映射：CUDA0 / Vulkan0 / CPU（显式 vulkan 裸值
// 会选中枚举序第一块 GPU，双 GPU 机器上常是核显，见 §2.4 实测）。

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::Manager;

use super::config::RuntimeConfig;

/// 配置里的 moss_device 请求值；"" 归一为 Auto
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceRequest {
    Auto,
    Cpu,
    Cuda,
    Vulkan,
}

/// "auto" | "" → Auto；未知值保守归一为 Auto（config 白名单已限制取值）
pub fn normalize_device_request(raw: &str) -> DeviceRequest {
    match raw.trim().to_ascii_lowercase().as_str() {
        "cpu" => DeviceRequest::Cpu,
        "cuda" => DeviceRequest::Cuda,
        "vulkan" => DeviceRequest::Vulkan,
        _ => DeviceRequest::Auto,
    }
}

/// ggml Vulkan 枚举表的一行（info stderr）
#[derive(Debug, Clone, PartialEq)]
pub struct VkDeviceInfo {
    /// 枚举序号（ggml Vulkan<N> 的 N = 本字段）
    pub index: u32,
    /// 设备名（如 "NVIDIA GeForce RTX 4060 Laptop GPU"）
    pub name: String,
    /// 统一内存（iGPU）：显存与系统内存共享，批大时会挤爆
    pub uma: bool,
}

/// L2 解析结果：实际生效后端 + Vulkan 枚举表（落点复核用）
#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveBackend {
    /// 如 "CUDA0" / "Vulkan1" / "CPU"
    pub backend: String,
    pub vk_devices: Vec<VkDeviceInfo>,
}

/// 预构建 cuda 版的最低可运行算力（×10）：`package_moss_flavors.ps1` 的
/// `-CudaArch 75;86;89` 只编了 sm_75/86/89 的内核，且 PTX 只向**更新的**架构 JIT
///（向后兼容是单向的）。sm < 7.5 的卡（GTX 10xx / Maxwell）加载 cuda 版会直接
/// 报 `no kernel image is available for execution on the device`——不是变慢，是失败。
pub const MIN_CUDA_COMPUTE_CAP: u32 = 75;

/// L0：机器能力探测结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MachineCaps {
    pub cuda: bool,
    pub vulkan: bool,
    /// 首块 NVIDIA GPU 的算力（×10，如 89 = 8.9；nvidia-smi 的 CUDA0 即第一块）。
    /// None = 取不到（无 nvidia-smi / 老驱动不支持 compute_cap 查询 / 卡名认不出）
    pub cuda_cc: Option<u32>,
}

impl MachineCaps {
    /// cuda 版能否在本机跑：要有 NVIDIA 卡，且算力 ≥ 7.5。
    /// 算力**未知**时放行（乐观策略）：预构建目标是 75 起，多数 PC 是 Turing 及以后；
    /// 拦错的代价（有卡却禁止用户用 cuda）比放错的代价（L2 报一句英文错误）更大。
    pub fn cuda_runnable(&self) -> bool {
        self.cuda
            && self
                .cuda_cc
                .map(|cc| cc >= MIN_CUDA_COMPUTE_CAP)
                .unwrap_or(true)
    }
}

/// L1：资产一致性（runtime/bin 内的标记与 DLL）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetInfo {
    /// `.backend` 标记文件内容（"cuda" / "vulkan" / None）
    pub marker: Option<String>,
    pub has_cuda_dll: bool,
    pub has_vulkan_dll: bool,
}

/// 判定结果（给自检面板展示）
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    /// 实际生效后端（L2 实证值，如 "CUDA0"）；Reject 时为空
    pub backend: String,
    /// 决策依据（人话，给欢迎页自检/设置页展示）
    pub reason: String,
    /// 决策来源："l2" 实证 / "l1" 资产推断 / "requested" 显式指定
    pub source: &'static str,
}

// ─── 解析（纯函数，可单测）───────────────────────────────

/// 从 `info` 的 stderr 提取实际生效后端与 Vulkan 枚举表。
/// 关键点：必须限定 `[mt I]` 行——ggml 静默降级时会同时打
/// `[mt W] backend: MTD_DEVICE=xxx requested but no matching device registered ?falling back`，
/// 贪婪匹配会把警告当结果（实测踩过）。
pub fn parse_info_stderr(stderr: &str) -> Option<EffectiveBackend> {
    let mut backend = None;
    for line in stderr.lines() {
        if let Some(pos) = line.find("[mt I] backend:") {
            // 取行尾到行首的值（Trim 掉 \r、空格），并容忍行内后缀
            let rest = line[pos + "[mt I] backend:".len()..].trim();
            if !rest.is_empty() {
                backend = Some(rest.to_string());
                break; // 取第一条实证行（启动日志只打一次）
            }
        }
    }
    let backend = backend?;
    Some(EffectiveBackend {
        backend,
        vk_devices: parse_vulkan_devices(stderr),
    })
}

/// 解析 ggml 的 Vulkan 枚举表：
/// `ggml_vulkan: 0 = Intel(R) ... | uma: 1 | fp16: 1 | ...`
pub fn parse_vulkan_devices(stderr: &str) -> Vec<VkDeviceInfo> {
    let mut devices = Vec::new();
    for line in stderr.lines() {
        // 只认 "ggml_vulkan: <数字> = ..." 的枚举行
        let Some(rest) = line.trim().strip_prefix("ggml_vulkan:") else {
            continue;
        };
        let rest = rest.trim();
        let Some((num, tail)) = rest.split_once('=') else {
            continue;
        };
        let Ok(index) = num.trim().parse::<u32>() else {
            continue;
        };
        let uma = tail.contains("uma: 1");
        // 设备名 = 等号后到第一个 '|' 之前
        let name = tail.split('|').next().unwrap_or("").trim().to_string();
        if !name.is_empty() {
            devices.push(VkDeviceInfo { index, name, uma });
        }
    }
    devices
}

/// 解析 `vulkaninfo --summary` 的 Devices 段，返回去重后的 GPU 名。
/// 本机实测：Intel 核显在 multi-plane 场景会被枚举 3 次（GPU0/2/3 同 UUID），
/// 按 deviceName+driverInfo 去重后只剩 1 条 Intel；顺序 = 枚举序。
pub fn parse_vulkaninfo_gpus(summary: &str) -> Vec<String> {
    // 先按 "GPU<N>:" 分块收集 (deviceName, driverInfo)，再按键去重
    let mut entries: Vec<(String, String)> = Vec::new();
    let mut name = String::new();
    let mut driver = String::new();
    let mut have_name = false;
    for line in summary.lines() {
        let trimmed = line.trim();
        // 块头：GPU0: / GPU1: ...（限长防误吞正文里的 "GPU" 字样）
        if trimmed.starts_with("GPU") && trimmed.ends_with(':') && trimmed.len() <= 8 {
            if have_name {
                entries.push((name.clone(), driver.clone()));
            }
            name.clear();
            driver.clear();
            have_name = false;
        } else if let Some(v) = trimmed.strip_prefix("deviceName") {
            name = value_after_eq(v).to_string();
            have_name = true;
        } else if let Some(v) = trimmed.strip_prefix("driverInfo") {
            driver = value_after_eq(v).to_string();
        }
    }
    if have_name {
        entries.push((name, driver));
    }
    // 去重：deviceName+driverInfo 相同视为同一设备（本机核显被枚举 3 次）
    let mut seen: Vec<(String, String)> = Vec::new();
    let mut out: Vec<String> = Vec::new();
    for (n, d) in entries {
        if !seen.iter().any(|(sn, sd)| sn == &n && sd == &d) {
            seen.push((n.clone(), d));
            out.push(n);
        }
    }
    out
}

fn value_after_eq(v: &str) -> &str {
    v.trim_start_matches(['=', ' ']).trim()
}

// ─── L0 / L1 探测（有子进程/IO，成本：nvidia-smi ~50ms，vulkaninfo ~200ms）──

pub fn probe_machine() -> MachineCaps {
    let (cuda, cuda_cc) = probe_nvidia();
    // Vulkan：运行时 loader（驱动提供）+ vulkaninfo 能枚举到 GPU
    let vulkan = vulkan_loader_present() && vulkaninfo_gpus_nonempty();
    MachineCaps { cuda, vulkan, cuda_cc }
}

/// NVIDIA 探测：两段式。先问 `compute_cap`（需要驱动 ≥ 510），拿不到再退回
/// `nvidia-smi -L` 的卡名映射（老卡老驱动也能判个八九不离十）。
fn probe_nvidia() -> (bool, Option<u32>) {
    if let Ok(o) = Command::new("nvidia-smi")
        .args(["--query-gpu=compute_cap", "--format=csv,noheader"])
        .output()
    {
        if o.status.success() {
            let text = String::from_utf8_lossy(&o.stdout);
            if let Some(cc) = parse_compute_cap(&text) {
                return (true, Some(cc));
            }
            // 成功但值是 N/A（驱动太老）：继续走 -L 的卡名兜底
        }
    }
    match Command::new("nvidia-smi").arg("-L").output() {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            (true, smi_list_first_name(&text).as_deref().and_then(cc_from_gpu_name))
        }
        _ => (false, None),
    }
}

/// 解析 `--query-gpu=compute_cap` 的输出（每行一块 GPU，如 "8.9"）：取**第一块**
/// （= ggml 的 CUDA0）。"8.9" → 89、"12.0" → 120；"N/A"/空/垃圾 → None。
pub fn parse_compute_cap(out: &str) -> Option<u32> {
    for line in out.lines() {
        let s = line.trim();
        if s.is_empty() {
            continue;
        }
        let (maj, min) = s.split_once('.').unwrap_or((s, "0"));
        let (Ok(maj), Ok(min)) = (maj.trim().parse::<u32>(), min.trim().parse::<u32>()) else {
            continue;
        };
        if maj == 0 {
            continue;
        }
        return Some(maj * 10 + min);
    }
    None
}

/// `nvidia-smi -L` 的第一块卡名：
/// `GPU 0: NVIDIA GeForce RTX 4060 Laptop GPU (UUID: GPU-...)` → `NVIDIA GeForce RTX 4060 Laptop GPU`
pub fn smi_list_first_name(list: &str) -> Option<String> {
    for line in list.lines() {
        let Some((_, rest)) = line.split_once(": ") else {
            continue;
        };
        // 卡名自身可能带括号，只切 " (UUID" 这个后缀
        let name = rest.split(" (UUID").next().unwrap_or(rest).trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    None
}

/// 卡名 → 算力（老驱动兜底）。只映射"一眼能定性"的型号，认不出返回 None（不拦人）。
pub fn cc_from_gpu_name(name: &str) -> Option<u32> {
    let n = name.to_ascii_uppercase();
    // "RTX 4060" / "GTX 1650" → 取型号数字，再按百位分代（4090/100 = 40）
    let series = ["RTX ", "GTX "].iter().find_map(|key| {
        let i = n.find(key)? + key.len();
        n[i..]
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse::<u32>()
            .ok()
    });
    if let Some(series) = series {
        return match series / 100 {
            50 => Some(120), // Blackwell
            40 => Some(89),  // Ada
            30 => Some(86),  // Ampere
            20 => Some(75),  // Turing
            16 => Some(75),  // GTX 1650/1660（Turing）
            10 => Some(61),  // Pascal
            9 => Some(52),   // Maxwell
            _ => None,
        };
    }
    // 数据中心 / 专业卡（名字里没有 RTX/GTX 前缀）
    if n.contains("TITAN V") {
        return Some(70);
    }
    if n.contains("V100") {
        return Some(70);
    }
    if n.contains("T4") {
        return Some(75);
    }
    if n.contains("A100") {
        return Some(80);
    }
    if n.contains("H100") {
        return Some(90);
    }
    None
}

fn vulkan_loader_present() -> bool {
    let sysroot = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
    Path::new(&sysroot).join("System32").join("vulkan-1.dll").exists()
}

fn vulkaninfo_gpus_nonempty() -> bool {
    Command::new("vulkaninfo")
        .arg("--summary")
        .output()
        .map(|o| {
            o.status.success()
                && !parse_vulkaninfo_gpus(&String::from_utf8_lossy(&o.stdout)).is_empty()
        })
        .unwrap_or(false)
}

/// L1：`.backend` 标记 + DLL 名（二进制的同目录）
pub fn probe_assets(binary: &Path) -> AssetInfo {
    let dir = binary.parent().unwrap_or_else(|| Path::new("."));
    let marker = std::fs::read_to_string(dir.join(".backend"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let has = |name: &str| dir.join(name).is_file();
    AssetInfo {
        marker,
        has_cuda_dll: has("ggml-cuda.dll"),
        has_vulkan_dll: has("ggml-vulkan.dll"),
    }
}

// ─── 判定（§3.4 矩阵，纯函数）────────────────────────────

/// 三级判定。Ok = 允许使用（backend 为实际生效后端）；Err = 硬报错文案
///（显式请求不可用，含"装对应后端"或"改回 auto"的可操作指引）。
pub fn decide(
    req: DeviceRequest,
    eff: Option<&EffectiveBackend>,
    assets: &AssetInfo,
    caps: &MachineCaps,
) -> Result<Decision, String> {
    match req {
        // CPU 后端（ggml-cpu）永远注册，无需 L2 实证
        DeviceRequest::Cpu => Ok(Decision {
            backend: "CPU".to_string(),
            reason: "已请求 cpu：CPU 后端始终可用".to_string(),
            source: "requested",
        }),
        // 老卡（算力 < 7.5）即使装了 cuda 版也必然 "no kernel image"：在 L1 就硬报错，
        // 别让它走到 L2 用一句 ggml 的英文报错糊过去（L2 那行对用户毫无信息量）
        DeviceRequest::Cuda => {
            // 注意先判 caps.cuda：没有 NVIDIA 卡是另一回事（"未检测到 GPU"），
            // 交给 verify_explicit 的 reject_message 说
            if caps.cuda && !caps.cuda_runnable() {
                return Err(old_card_message(caps));
            }
            verify_explicit("cuda", "CUDA", eff, assets, caps)
        }
        DeviceRequest::Vulkan => verify_explicit("vulkan", "Vulkan", eff, assets, caps),
        // auto：以 L2 实证为准；probe 不可用时退回资产推断
        DeviceRequest::Auto => match eff {
            Some(e) => {
                let mut reason = format!("auto：ggml 自动选优，实测生效 {}", e.backend);
                // 显式 vulkan 落在核显（枚举序第一块）时提示（§2.4 实测）
                if e.backend.starts_with("Vulkan") {
                    if let Some(idx) = e.backend.strip_prefix("Vulkan").and_then(|s| s.parse::<u32>().ok()) {
                        if let Some(dev) = e.vk_devices.iter().find(|d| d.index == idx) {
                            if dev.uma {
                                reason.push_str(&format!(
                                    "（{}，统一内存 iGPU，批大时可能变慢）", dev.name
                                ));
                            }
                        }
                    }
                }
                Ok(Decision { backend: e.backend.clone(), reason, source: "l2" })
            }
            None => {
                let d = recommend_from_assets(assets);
                Ok(Decision {
                    backend: d.backend.clone(),
                    reason: format!("未实证（info 无 backend 行），按已装资产推断：{}", d.reason),
                    source: "l1",
                })
            }
        },
    }
}

/// 显式请求的可用性校验（cuda / vulkan 共用）。
/// req_raw 用于报错文案；expect 是 L2 backend 行的前缀（"CUDA"/"Vulkan"）。
fn verify_explicit(
    req_raw: &str,
    expect: &str,
    eff: Option<&EffectiveBackend>,
    assets: &AssetInfo,
    caps: &MachineCaps,
) -> Result<Decision, String> {
    if let Some(e) = eff {
        if e.backend.starts_with(expect) {
            let mut reason = format!("已请求 {}，实测生效 {}", req_raw, e.backend);
            if expect == "Vulkan" {
                // 裸 vulkan 常落核显（枚举序第一块），实测提醒（§2.4）
                if let Some(idx) = e.backend.strip_prefix("Vulkan").and_then(|s| s.parse::<u32>().ok()) {
                    if let Some(dev) = e.vk_devices.iter().find(|d| d.index == idx) {
                        if dev.uma {
                            reason.push_str(&format!("（{}，统一内存 iGPU，建议 moss_device 留空）", dev.name));
                        }
                    }
                }
            }
            return Ok(Decision { backend: e.backend.clone(), reason, source: "requested" });
        }
        // 实测后端不符 → 硬报错（ggml 会静默降级，这里必须拦下）
        return Err(reject_message(req_raw, &e.backend, caps));
    }
    // info 没给 backend 行（老版本二进制）：退回资产推断，能对上就放行。
    // 判据优先级：.backend 标记（上次成功构建写入）> DLL 存在性——标记存在但与请求
    // 不符 ⇒ DLL 可能是切后端时未清的陈旧残留，不得单方放行（PR48 审查）。
    let dll_ok = (expect == "CUDA" && assets.has_cuda_dll)
        || (expect == "Vulkan" && assets.has_vulkan_dll);
    match assets.marker.as_deref() {
        Some(m) if !m.eq_ignore_ascii_case(req_raw) => Err(reject_message(
            req_raw,
            &format!("（info 无 backend 行，.backend 标记为 {m}）"),
            caps,
        )),
        Some(m) => Ok(Decision {
            backend: req_raw.to_string(),
            reason: format!("info 未实证 backend 行，按 .backend 标记（{m}）放行"),
            source: "l1",
        }),
        None if dll_ok => Ok(Decision {
            backend: req_raw.to_string(),
            reason: format!(
                "info 未实证 backend 行，按已装资产（发现 {}）放行",
                if expect == "CUDA" { "ggml-cuda.dll" } else { "ggml-vulkan.dll" }
            ),
            source: "l1",
        }),
        None => Err(reject_message(req_raw, "（未实证，info 无 backend 行且无对应资产）", caps)),
    }
}

/// 算力给人看的形式：Some(89) → "8.9"，None → "未知"
fn fmt_cc(cc: Option<u32>) -> String {
    match cc {
        Some(cc) => format!("{}.{}", cc / 10, cc % 10),
        None => "未知".to_string(),
    }
}

/// 老卡的拒绝文案：这条不属于"没装后端"，而是"装了也跑不起来"，指引也不同
/// （不要去重建 cuda 版，直接换 vulkan/cpu）
fn old_card_message(caps: &MachineCaps) -> String {
    let cc = caps.cuda_cc.unwrap_or(0);
    format!(
        "已配置 moss_device=cuda，但本机 NVIDIA 显卡算力 {}.{} 低于预构建 cuda 版的最低要求 {}.{}：\
         预构建只含 sm_75/86/89 内核，老卡会直接报 \"no kernel image is available for execution on the device\"。\
         请把 moss_device 改为 {}{}",
        cc / 10,
        cc % 10,
        MIN_CUDA_COMPUTE_CAP / 10,
        MIN_CUDA_COMPUTE_CAP % 10,
        if caps.vulkan { "vulkan" } else { "cpu" },
        if caps.vulkan { "（本机 Vulkan 可用，三厂通吃）" } else { "（本机 Vulkan 也不可用）" },
    )
}

/// 拒绝文案：按 L0 给"装对应后端"或"改回 auto"的指引（§3.4）
fn reject_message(req_raw: &str, actual: &str, caps: &MachineCaps) -> String {
    // NVIDIA 有卡但太老：别说"去装 cuda 版"，那是条死路
    if req_raw == "cuda" && caps.cuda && !caps.cuda_runnable() {
        return old_card_message(caps);
    }
    let (build_cmd, has_hw, hw_hint) = match req_raw {
        "cuda" => ("cuda", caps.cuda, "nvidia-smi 未检测到 NVIDIA GPU"),
        _ => ("vulkan", caps.vulkan, "vulkaninfo 未检测到 Vulkan GPU（驱动缺失？）"),
    };
    if has_hw {
        format!(
            "已配置 moss_device={req_raw}，但实测生效后端为 {actual}：\
             请运行 scripts/build_moss.ps1 -Backend {build_cmd} 重装对应版本，\
             或把 moss_device 改为空（auto）",
            req_raw = req_raw,
            actual = actual,
            build_cmd = build_cmd,
        )
    } else {
        format!(
            "已配置 moss_device={req_raw}，但本机 {hw_hint}：\
             请把 moss_device 改为空（auto）或 cpu",
            req_raw = req_raw,
            hw_hint = hw_hint,
        )
    }
}

/// 资产层推荐（§3.4 auto 行的 L1/L0 版，给"该装哪个后端"的自检提示用）：
/// 已装 CUDA → cuda；否则已装 Vulkan → vulkan；否则 cpu
pub fn recommend_from_assets(assets: &AssetInfo) -> Decision {
    let marker = assets.marker.as_deref().unwrap_or("");
    if marker.eq_ignore_ascii_case("cuda") || assets.has_cuda_dll {
        Decision {
            backend: "cuda".to_string(),
            reason: "已装 CUDA 版 moss（.backend 标记 / ggml-cuda.dll）".to_string(),
            source: "l1",
        }
    } else if marker.eq_ignore_ascii_case("vulkan") || assets.has_vulkan_dll {
        Decision {
            backend: "vulkan".to_string(),
            reason: "已装 Vulkan 版 moss（.backend 标记 / ggml-vulkan.dll）".to_string(),
            source: "l1",
        }
    } else {
        Decision {
            backend: "cpu".to_string(),
            reason: "未检测到 GPU 版 moss，自动用 CPU".to_string(),
            source: "l1",
        }
    }
}

/// 推荐后端（资产 × 算力）：给"该装哪个后端"的自检提示用。
/// 纯函数，与 probe_backend 内的推荐共用，便于单测。
pub fn recommend(assets: &AssetInfo, caps: &MachineCaps) -> (String, String) {
    let rec = recommend_from_assets(assets);
    let cuda_pref = caps.cuda && caps.cuda_runnable();
    if rec.backend == "cuda" && !cuda_pref {
        // 已装 cuda 版但本机卡太老（或干脆没 NVIDIA 卡）：推荐降级到 Vulkan/CPU
        let b = if caps.vulkan { "vulkan" } else { "cpu" };
        return (
            b.to_string(),
            format!(
                "已装 CUDA 版 moss，但本机显卡算力 {} 低于预构建 cuda 版要求 {}，跑不起来；推荐 {} 版",
                fmt_cc(caps.cuda_cc),
                fmt_cc(Some(MIN_CUDA_COMPUTE_CAP)),
                b
            ),
        );
    }
    if rec.backend == "cpu" && (caps.cuda || caps.vulkan) {
        let b = if cuda_pref {
            "cuda"
        } else if caps.vulkan {
            "vulkan"
        } else {
            "cpu"
        };
        return (
            b.to_string(),
            format!(
                "本机有 {} GPU，建议安装 {} 版 moss（当前只装了 CPU 版）",
                if b == "cuda" { "NVIDIA" } else { "Vulkan" },
                b
            ),
        );
    }
    (rec.backend, rec.reason)
}

// ─── 探测报告（前端设备下拉 + 硬报错展示）───────────────

/// 设备探测报告：前端据此渲染「推理设备」下拉、可用性提示与硬报错
#[derive(Debug, Clone, Serialize)]
pub struct BackendProbeReport {
    /// 配置里的 moss_device 原值（"" = auto）
    pub configured_device: String,
    /// 本次判定实际请求的设备："auto" / "cpu" / "cuda" / "vulkan"
    pub requested: String,
    /// 请求是否可用：显式请求不可用 = false（硬报错，绝不静默降级）；auto 恒 true
    pub available: bool,
    /// 实际生效后端（"CUDA0" / "Vulkan1" / "CPU"）；不可用或未实证时为空
    pub backend: String,
    /// 判定依据（人话，给自检面板/设置页展示）
    pub reason: String,
    /// 判定来源："l2" 实证 / "l1" 资产推断 / "requested" 显式指定 / "reject" 硬报错
    pub source: String,
    /// 推荐后端（"cuda" / "vulkan" / "cpu"）与推荐理由
    pub recommendation: String,
    pub recommendation_reason: String,
    /// 本机能力：NVIDIA 驱动可用 / Vulkan loader + GPU 可枚举
    pub machine_cuda: bool,
    pub machine_vulkan: bool,
    /// 首块 NVIDIA GPU 的算力（"8.9"）；None = 未取得（无 NVIDIA / 老驱动 / 卡名认不出）
    pub cuda_compute_cap: Option<String>,
    /// 预构建 cuda 版能否在本机运行（算力 < 7.5 ⇒ false；算力未知 ⇒ true 乐观放行）
    pub cuda_runnable: bool,
    /// Vulkan 枚举到的 GPU 名（vulkaninfo）
    pub gpus: Vec<String>,
    /// manifest 是否找到：false = 没找到 manifest ⇒ 前端按"四项全可选"退化展示
    pub manifest_found: bool,
    /// manifest 里 id=="moss" 的条目声明的 flavor（去重、小写）
    pub manifest_flavors: Vec<String>,
    /// 硬报错信息（显式请求不可用 / MOSS 未安装时非空）
    pub error: Option<String>,
}

/// manifest 里可安装的 MOSS 后端集合
#[derive(Debug, Clone, PartialEq)]
pub struct ManifestFlavors {
    /// manifest 是否找到
    pub found: bool,
    /// id=="moss" 的条目声明的 flavor（去重、小写，保持出现顺序）
    pub flavors: Vec<String>,
}

/// 读取 manifest 中 id=="moss" 的 flavor 集合。
/// manifest 未找到 → found=false（前端退化显示全部四项）；
/// 找到但没有 moss 条目 → found=true + 空集合（前端不给选任何具体 flavor）。
pub fn read_manifest_flavors(manifest_path: Option<&Path>) -> ManifestFlavors {
    let Some(path) = manifest_path else {
        return ManifestFlavors { found: false, flavors: Vec::new() };
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return ManifestFlavors { found: false, flavors: Vec::new() };
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return ManifestFlavors { found: false, flavors: Vec::new() };
    };
    let mut flavors: Vec<String> = Vec::new();
    if let Some(items) = json.get("items").and_then(|v| v.as_array()) {
        for item in items {
            let is_moss = item.get("id").and_then(|v| v.as_str()) == Some("moss");
            if !is_moss {
                continue;
            }
            // flavor 可选；缺失的条目无法判定具体后端，跳过
            let Some(flavor) = item.get("flavor").and_then(|v| v.as_str()) else {
                continue;
            };
            let f = flavor.trim().to_ascii_lowercase();
            if !f.is_empty() && !flavors.contains(&f) {
                flavors.push(f);
            }
        }
    }
    ManifestFlavors { found: true, flavors }
}

/// 按候选路径顺序找 manifest 文件（第一个存在的胜出）
pub fn find_manifest(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|p| p.is_file()).cloned()
}

/// 探测 MOSS 后端：L0 机器能力 + L1 资产 + L2 实证（`info <model>` 的 backend 行），
/// 再按 §3.4 矩阵判定。device_override 非空时按该设备判定（用于下拉预览）。
pub fn probe_backend(
    runtime_dir: &Path,
    config: &RuntimeConfig,
    device_override: Option<&str>,
    manifest_candidates: &[PathBuf],
) -> BackendProbeReport {
    let configured_device = config.moss_device.clone();
    let requested_raw = device_override
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| configured_device.clone());
    let req = normalize_device_request(&requested_raw);

    let (binary, model) = super::moss_paths(config, runtime_dir);
    let caps = probe_machine();
    let assets = probe_assets(&binary);

    let (backend, reason, source, error) = if !binary.is_file() || !model.is_file() {
        let msg = format!(
            "MOSS 未就绪：可执行文件 {} / 模型 {} 不存在（先安装 MOSS 运行时）",
            binary.display(),
            model.display()
        );
        (String::new(), msg.clone(), "reject".to_string(), Some(msg))
    } else {
        match run_info(&binary, &model, req) {
            Ok(stderr) => {
                let eff = parse_info_stderr(&stderr);
                match decide(req, eff.as_ref(), &assets, &caps) {
                    Ok(d) => (d.backend, d.reason, d.source.to_string(), None),
                    Err(msg) => (String::new(), msg.clone(), "reject".to_string(), Some(msg)),
                }
            }
            Err(msg) => (String::new(), msg.clone(), "reject".to_string(), Some(msg)),
        }
    };

    // 推荐后端：资产层说话；但要过一遍算力——装了 cuda 版而卡太老时，推荐得反过来
    // （这正是"CUDA → Vulkan → CPU"层级在推荐侧的体现：Vulkan 是三厂 + 老卡的通吃档）
    let (recommendation, recommendation_reason) = recommend(&assets, &caps);

    let manifest = read_manifest_flavors(find_manifest(manifest_candidates).as_deref());

    BackendProbeReport {
        configured_device,
        requested: requested_raw,
        available: error.is_none(),
        backend,
        reason,
        source,
        recommendation,
        recommendation_reason,
        machine_cuda: caps.cuda,
        machine_vulkan: caps.vulkan,
        cuda_compute_cap: caps.cuda_cc.map(|cc| format!("{}.{}", cc / 10, cc % 10)),
        cuda_runnable: caps.cuda_runnable(),
        gpus: probe_vulkan_gpus(),
        manifest_found: manifest.found,
        manifest_flavors: manifest.flavors,
        error,
    }
}

/// 跑 `info <model>` 拿 stderr（`[mt I] backend:` 行在其内）。
/// 显式设备才设 MTD_DEVICE——这样实测到的后端与转写时真实生效的一致。
fn run_info(binary: &Path, model: &Path, req: DeviceRequest) -> Result<String, String> {
    let mut cmd = Command::new(binary);
    cmd.arg("info").arg(model);
    match req {
        DeviceRequest::Auto => {}
        DeviceRequest::Cpu => {
            cmd.env("MTD_DEVICE", "cpu");
        }
        DeviceRequest::Cuda => {
            cmd.env("MTD_DEVICE", "cuda");
        }
        DeviceRequest::Vulkan => {
            cmd.env("MTD_DEVICE", "vulkan");
        }
    }
    let out = cmd
        .output()
        .map_err(|e| format!("无法启动 MOSS：{}", e))?;
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    if !out.status.success() {
        return Err(format!(
            "MOSS info 探测失败（{}）：{}",
            out.status,
            stderr.trim()
        ));
    }
    Ok(stderr)
}

/// Vulkan 枚举到的 GPU 名（vulkaninfo --summary）；无 Vulkan 时为空
pub fn probe_vulkan_gpus() -> Vec<String> {
    Command::new("vulkaninfo")
        .arg("--summary")
        .output()
        .map(|o| {
            if o.status.success() {
                parse_vulkaninfo_gpus(&String::from_utf8_lossy(&o.stdout))
            } else {
                Vec::new()
            }
        })
        .unwrap_or_default()
}

/// Tauri 命令：探测 MOSS 后端（设备下拉 + 硬报错展示）
///
/// device 省略时按当前配置的 moss_device 判定；传入时按该设备判定（下拉预览）。
#[tauri::command]
pub fn probe_moss_backend(
    app: tauri::AppHandle,
    device: Option<String>,
) -> BackendProbeReport {
    let runtime_dir = super::config::resolve_runtime_dir(
        app.path().app_data_dir().ok().map(|d| d.join("runtime")),
    );
    let config = RuntimeConfig::load(&runtime_dir).unwrap_or_default();
    probe_backend(&runtime_dir, &config, device.as_deref(), &manifest_candidates(&app, &runtime_dir))
}

/// manifest 候选路径：runtime 目录内 → Tauri 打包布局（exe 同级 runtime/ 或
/// exe 同级 resources/runtime/）→ 开发期布局（src-tauri/resources/runtime/）
/// → 显式 resource_dir
pub(crate) fn manifest_candidates(app: &tauri::AppHandle, runtime_dir: &Path) -> Vec<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let resource_dir = app.path().resource_dir().ok();
    manifest_candidates_for(runtime_dir, exe_dir.as_deref(), resource_dir.as_deref())
}

/// manifest 候选路径（纯函数，便于单测）
///
/// 注意 `bundle.resources` 用**数组**形式时 Tauri 保留原目录结构
/// （`resources/runtime/manifest.json` → `$RESOURCES/resources/runtime/manifest.json`），
/// 所以 `*/resources/runtime/manifest.json` 这两条必须留着，否则打包后读不到清单、
/// 前端会一直走"四项全可选"的退化分支。
fn manifest_candidates_for(
    runtime_dir: &Path,
    exe_dir: Option<&Path>,
    resource_dir: Option<&Path>,
) -> Vec<PathBuf> {
    let mut candidates = vec![runtime_dir.join("manifest.json")];
    if let Some(dir) = exe_dir {
        candidates.push(dir.join("runtime").join("manifest.json"));
        candidates.push(dir.join("resources").join("runtime").join("manifest.json"));
        candidates.push(dir.join("../../resources/runtime/manifest.json"));
    }
    if let Some(res) = resource_dir {
        candidates.push(res.join("runtime").join("manifest.json"));
        candidates.push(res.join("resources").join("runtime").join("manifest.json"));
    }
    candidates
}

// ─── 单测（夹具 = 实测 stderr / vulkaninfo 输出）─────────

#[cfg(test)]
mod tests {
    use super::*;

    const STDERR_CUDA: &str = "[mt I] loading model meta\r\n\
        [mt I] backend: CUDA0\r\n\
        [mt I] model ready";
    const STDERR_VULKAN_DEGRADE: &str = "ggml_vulkan: Found 2 Vulkan devices:\r\n\
        ggml_vulkan: 0 = Intel(R) RaptorLake-S Mobile Graphics Controller (Intel Corporation) | uma: 1 | fp16: 1 | bf16: 0\r\n\
        ggml_vulkan: 1 = NVIDIA GeForce RTX 4060 Laptop GPU (NVIDIA) | uma: 0 | fp16: 1 | bf16: 0 | warp size: 32\r\n\
        [mt W] backend: MTD_DEVICE=vulkan requested but no matching device registered ?falling back\r\n\
        [mt I] backend: Vulkan0\r\n\
        [mt I] model ready";
    const STDERR_CPU: &str = "[mt I] backend: CPU\r\n[mt I] model ready";
    const STDERR_NO_BACKEND: &str = "[mt I] loading model meta\r\n[mt I] model ready";

    #[test]
    fn parse_backend_cuda0() {
        let eff = parse_info_stderr(STDERR_CUDA).unwrap();
        assert_eq!(eff.backend, "CUDA0");
        assert!(eff.vk_devices.is_empty());
    }

    #[test]
    fn parse_backend_ignores_degrade_warning() {
        // 关键：贪婪匹配会把 [mt W] 的 falling back 行当结果，必须只认 [mt I]
        let eff = parse_info_stderr(STDERR_VULKAN_DEGRADE).unwrap();
        assert_eq!(eff.backend, "Vulkan0");
    }

    #[test]
    fn parse_backend_none_when_missing() {
        assert!(parse_info_stderr(STDERR_NO_BACKEND).is_none());
        assert!(parse_info_stderr("").is_none());
    }

    #[test]
    fn parse_vulkan_enum_table() {
        let eff = parse_info_stderr(STDERR_VULKAN_DEGRADE).unwrap();
        assert_eq!(eff.vk_devices.len(), 2);
        assert_eq!(eff.vk_devices[0].index, 0);
        assert!(eff.vk_devices[0].uma, "核显应识别为统一内存");
        // ggml 枚举行名字段含厂商后缀（"…(NVIDIA)"），原样保留
        assert_eq!(
            eff.vk_devices[1].name,
            "NVIDIA GeForce RTX 4060 Laptop GPU (NVIDIA)"
        );
        assert!(!eff.vk_devices[1].uma);
    }

    #[test]
    fn normalize_request() {
        assert_eq!(normalize_device_request(""), DeviceRequest::Auto);
        assert_eq!(normalize_device_request("auto"), DeviceRequest::Auto);
        assert_eq!(normalize_device_request("CPU"), DeviceRequest::Cpu);
        assert_eq!(normalize_device_request("cuda"), DeviceRequest::Cuda);
        assert_eq!(normalize_device_request("vulkan "), DeviceRequest::Vulkan);
        assert_eq!(normalize_device_request("gpu:0"), DeviceRequest::Auto);
    }

    fn caps(cuda: bool, vulkan: bool) -> MachineCaps {
        // 算力未知（None）：等于"不按算力分流"，老用例语义不变
        MachineCaps { cuda, vulkan, cuda_cc: None }
    }
    fn caps_cc(cuda: bool, vulkan: bool, cc: Option<u32>) -> MachineCaps {
        MachineCaps { cuda, vulkan, cuda_cc: cc }
    }
    fn assets(marker: Option<&str>, cu: bool, vk: bool) -> AssetInfo {
        AssetInfo {
            marker: marker.map(|s| s.to_string()),
            has_cuda_dll: cu,
            has_vulkan_dll: vk,
        }
    }

    #[test]
    fn decide_explicit_ok() {
        let eff = parse_info_stderr(STDERR_CUDA);
        let a = assets(Some("cuda"), true, false);
        let d = decide(DeviceRequest::Cuda, eff.as_ref(), &a, &caps(true, true)).unwrap();
        assert_eq!(d.backend, "CUDA0");
        assert_eq!(d.source, "requested");
    }

    #[test]
    fn decide_explicit_mismatch_is_hard_error() {
        // 请求 cuda 但实测 Vulkan（装错后端）→ 拒绝
        let eff = parse_info_stderr(STDERR_VULKAN_DEGRADE);
        let a = assets(Some("vulkan"), false, true);
        let err = decide(DeviceRequest::Cuda, eff.as_ref(), &a, &caps(true, true)).unwrap_err();
        assert!(err.contains("moss_device=cuda"), "{}", err);
        assert!(err.contains("build_moss.ps1 -Backend cuda"), "{}", err);
    }

    #[test]
    fn decide_explicit_vulkan_same_family_warns_uma() {
        // 显式 vulkan + 实测 Vulkan0（核显）= 同族可用：只警告落点，不拒绝
        //（§3.4：跨族降级才是硬错误；落点问题走 uma 提示 + bootstrap 落点复核）
        let eff = parse_info_stderr(STDERR_VULKAN_DEGRADE);
        let a = assets(Some("vulkan"), false, true);
        let d = decide(DeviceRequest::Vulkan, eff.as_ref(), &a, &caps(true, true)).unwrap();
        assert_eq!(d.backend, "Vulkan0");
        assert_eq!(d.source, "requested");
        assert!(d.reason.contains("统一内存"), "{}", d.reason);
    }

    #[test]
    fn decide_explicit_vulkan_cross_family_is_hard_error() {
        // 请求 vulkan 但实测 CUDA（装错后端 / ggml 降级到别的后端）→ 硬报错
        let eff = parse_info_stderr(STDERR_CUDA);
        let a = assets(Some("cuda"), true, false);
        let err = decide(DeviceRequest::Vulkan, eff.as_ref(), &a, &caps(true, true)).unwrap_err();
        assert!(err.contains("moss_device=vulkan"), "{}", err);
        assert!(err.contains("build_moss.ps1 -Backend vulkan"), "{}", err);
    }

    #[test]
    fn decide_explicit_cuda_no_hardware_hints_auto() {
        let eff = parse_info_stderr(STDERR_CUDA);
        let a = assets(Some("cuda"), true, false);
        let err = decide(DeviceRequest::Vulkan, eff.as_ref(), &a, &caps(true, false)).unwrap_err();
        assert!(err.contains("改为空（auto）或 cpu"), "{}", err);
    }

    #[test]
    fn decide_cpu_always_ok() {
        let a = assets(None, false, false);
        let d = decide(DeviceRequest::Cpu, None, &a, &caps(false, false)).unwrap();
        assert_eq!(d.backend, "CPU");
        assert_eq!(d.source, "requested");
    }

    #[test]
    fn decide_explicit_l1_fallback_marker_rules() {
        // info 无 backend 行（老版本二进制）时的 L1 放行判据（verify_explicit 的
        // eff=None 路径）：① marker 一致 → 放行；② marker 缺失但 DLL 在 → 放行，
        // 理由写明依据是 DLL；③ marker 与请求不符 → 拒绝——DLL 可能是切后端时
        // 未清的陈旧残留，不得单方放行
        let d = decide(DeviceRequest::Cuda, None, &assets(Some("cuda"), true, false), &caps(true, true)).unwrap();
        assert_eq!(d.source, "l1");
        assert!(d.reason.contains(".backend"), "{}", d.reason);

        let d = decide(DeviceRequest::Cuda, None, &assets(None, true, false), &caps(true, true)).unwrap();
        assert!(d.reason.contains("ggml-cuda.dll"), "{}", d.reason);

        let err = decide(DeviceRequest::Cuda, None, &assets(Some("vulkan"), false, true), &caps(true, true)).unwrap_err();
        assert!(err.contains(".backend 标记为 vulkan"), "{}", err);
    }

    #[test]
    fn decide_auto_uses_l2_and_warns_uma() {
        // auto 实测 Vulkan0 = 核显 → backend 记录 + 理由带 uma 提示
        let eff = parse_info_stderr(STDERR_VULKAN_DEGRADE);
        let a = assets(Some("vulkan"), false, true);
        let d = decide(DeviceRequest::Auto, eff.as_ref(), &a, &caps(true, true)).unwrap();
        assert_eq!(d.backend, "Vulkan0");
        assert_eq!(d.source, "l2");
        assert!(d.reason.contains("统一内存"), "{}", d.reason);
    }

    #[test]
    fn decide_auto_falls_back_to_assets_without_l2() {
        let a = assets(Some("cuda"), true, false);
        let d = decide(DeviceRequest::Auto, None, &a, &caps(true, false)).unwrap();
        assert_eq!(d.backend, "cuda");
        assert_eq!(d.source, "l1");
    }

    #[test]
    fn vulkaninfo_parse_dedupes_igpu() {
        // 本机实测：GPU0/2/3 同一 Intel 核显（driverInfo 101.5542），GPU1 = 4060
        let fixture = "\
GPU0:
\tdeviceName         = Intel(R) RaptorLake-S Mobile Graphics Controller
\tdriverInfo         = 101.5542
GPU1:
\tdeviceName         = NVIDIA GeForce RTX 4060 Laptop GPU
\tdriverInfo         = 572.83
GPU2:
\tdeviceName         = Intel(R) RaptorLake-S Mobile Graphics Controller
\tdriverInfo         = 101.5542
GPU3:
\tdeviceName         = Intel(R) RaptorLake-S Mobile Graphics Controller
\tdriverInfo         = 101.5542
";
        let gpus = parse_vulkaninfo_gpus(fixture);
        assert_eq!(gpus.len(), 2, "核显三连枚举应去重：{:?}", gpus);
        assert!(gpus.iter().any(|g| g.contains("4060")));
        assert!(gpus.iter().any(|g| g.contains("RaptorLake")));
    }

    #[test]
    fn vulkaninfo_parse_empty_when_no_devices() {
        assert!(parse_vulkaninfo_gpus("Vulkan Instance Version: 1.4.363\n\nDevices:\n========\n").is_empty());
    }

    // ─── manifest flavor 读取 ────────────────────────────

    /// 写临时 manifest 文件（测试用）
    fn write_temp_manifest(name: &str, body: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("gsa_manifest_{}_{}.json", name, std::process::id()));
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn manifest_missing_is_not_found() {
        let f = read_manifest_flavors(None);
        assert!(!f.found);
        assert!(f.flavors.is_empty());
    }

    #[test]
    fn manifest_collects_moss_flavors() {
        let path = write_temp_manifest(
            "flavors",
            r#"{"version":1,"generated_at":"2026-01-01","items":[
                {"id":"moss","flavor":"CUDA","url":"http://x/a.zip","sha256":"aa","size":1,"dest":"bin"},
                {"id":"moss","flavor":"cuda","url":"http://x/b.zip","sha256":"bb","size":1,"dest":"bin"},
                {"id":"moss","flavor":"vulkan","url":"http://x/c.zip","sha256":"cc","size":1,"dest":"bin"},
                {"id":"deps-ocr","flavor":"cpu","url":"http://x/d.zip","sha256":"dd","size":1,"dest":"deps"}
            ]}"#,
        );
        let f = read_manifest_flavors(Some(&path));
        let _ = std::fs::remove_file(&path);
        assert!(f.found);
        // 只取 moss 条目、flavor 小写去重
        assert_eq!(f.flavors, vec!["cuda".to_string(), "vulkan".to_string()]);
    }

    #[test]
    fn manifest_without_moss_items_yields_empty_set() {
        let path = write_temp_manifest(
            "nomoss",
            r#"{"version":1,"items":[{"id":"deps-ocr","flavor":"cpu","url":"u","sha256":"s","size":1,"dest":"d"}]}"#,
        );
        let f = read_manifest_flavors(Some(&path));
        let _ = std::fs::remove_file(&path);
        assert!(f.found);
        assert!(f.flavors.is_empty());
    }

    #[test]
    fn manifest_broken_json_is_not_found() {
        let path = write_temp_manifest("broken", "{not json");
        let f = read_manifest_flavors(Some(&path));
        let _ = std::fs::remove_file(&path);
        assert!(!f.found);
    }

    /// 打包布局：`bundle.resources` 数组形式保留目录结构 ⇒ 清单落在
    /// `$RESOURCES/resources/runtime/manifest.json`，候选路径必须能找到。
    #[test]
    fn manifest_candidates_find_bundled_layout() {
        let root = std::env::temp_dir().join(format!("gsa_bundle_{}", std::process::id()));
        let exe_dir = root.join("app");
        let res_dir = root.join("app");
        let res = res_dir.join("resources").join("runtime");
        std::fs::create_dir_all(&res).unwrap();
        let written = res.join("manifest.json");
        std::fs::write(
            &written,
            r#"{"version":1,"items":[{"id":"moss","flavor":"cpu","url":"u","sha256":"s","size":1,"dest":"bin"}]}"#,
        )
        .unwrap();

        let cands = manifest_candidates_for(&root.join("userdata"), Some(&exe_dir), Some(&res_dir));
        let found = find_manifest(&cands);
        let flavors = read_manifest_flavors(found.as_deref());
        let _ = std::fs::remove_dir_all(&root);

        assert!(found.is_some(), "打包布局下没找到清单，候选：{:?}", cands);
        assert!(flavors.found);
        assert_eq!(flavors.flavors, vec!["cpu".to_string()]);
    }

    /// 开发期布局：exe 在 `src-tauri/target/debug/` ⇒ `../../resources/runtime/manifest.json`
    #[test]
    fn manifest_candidates_find_dev_layout() {
        let root = std::env::temp_dir().join(format!("gsa_dev_{}", std::process::id()));
        let exe_dir = root.join("src-tauri").join("target").join("debug");
        let res = root.join("src-tauri").join("resources").join("runtime");
        std::fs::create_dir_all(&exe_dir).unwrap();
        std::fs::create_dir_all(&res).unwrap();
        std::fs::write(
            res.join("manifest.json"),
            r#"{"version":1,"items":[{"id":"moss","flavor":"vulkan","url":"u","sha256":"s","size":1,"dest":"bin"}]}"#,
        )
        .unwrap();

        let cands = manifest_candidates_for(&root.join("userdata"), Some(&exe_dir), None);
        let found = find_manifest(&cands);
        let flavors = read_manifest_flavors(found.as_deref());
        let _ = std::fs::remove_dir_all(&root);

        assert!(found.is_some(), "dev 布局下没找到清单，候选：{:?}", cands);
        assert_eq!(flavors.flavors, vec!["vulkan".to_string()]);
    }

    // ─── 算力分流（老卡 sm < 7.5）─────────────────────────

    #[test]
    fn parse_compute_cap_handles_real_output() {
        // 本机实测：nvidia-smi --query-gpu=compute_cap --format=csv,noheader → "8.9"
        assert_eq!(parse_compute_cap("8.9\r\n"), Some(89));
        assert_eq!(parse_compute_cap("7.5"), Some(75));
        assert_eq!(parse_compute_cap("12.0"), Some(120));
        // 多卡：取第一块（= ggml 的 CUDA0）
        assert_eq!(parse_compute_cap("6.1\n8.9\n"), Some(61));
        // 老驱动不支持该查询 → N/A / 空
        assert_eq!(parse_compute_cap("N/A\n"), None);
        assert_eq!(parse_compute_cap(""), None);
        assert_eq!(parse_compute_cap("garbage"), None);
    }

    #[test]
    fn cc_from_gpu_name_maps_families() {
        assert_eq!(cc_from_gpu_name("NVIDIA GeForce RTX 4060 Laptop GPU"), Some(89));
        assert_eq!(cc_from_gpu_name("NVIDIA GeForce RTX 3060"), Some(86));
        assert_eq!(cc_from_gpu_name("NVIDIA GeForce RTX 2080 Ti"), Some(75));
        assert_eq!(cc_from_gpu_name("NVIDIA GeForce GTX 1650"), Some(75));
        assert_eq!(cc_from_gpu_name("NVIDIA GeForce GTX 1080 Ti"), Some(61));
        assert_eq!(cc_from_gpu_name("NVIDIA GeForce GTX 960"), Some(52));
        assert_eq!(cc_from_gpu_name("Tesla V100-SXM2-16GB"), Some(70));
        // 认不出 → None（乐观放行，不拦人）
        assert_eq!(cc_from_gpu_name("Some Unknown Accelerator"), None);
    }

    #[test]
    fn smi_list_first_name_strips_uuid() {
        let list = "GPU 0: NVIDIA GeForce RTX 4060 Laptop GPU (UUID: GPU-1234)\r\n\
                    GPU 1: NVIDIA GeForce GTX 1650 (UUID: GPU-5678)\r\n";
        assert_eq!(
            smi_list_first_name(list).as_deref(),
            Some("NVIDIA GeForce RTX 4060 Laptop GPU")
        );
        assert_eq!(smi_list_first_name("No devices were found\n"), None);
    }

    #[test]
    fn cuda_runnable_matrix() {
        assert!(caps_cc(true, true, Some(89)).cuda_runnable());
        assert!(caps_cc(true, true, Some(75)).cuda_runnable());
        assert!(!caps_cc(true, true, Some(61)).cuda_runnable());
        assert!(!caps_cc(false, true, None).cuda_runnable(), "没 NVIDIA 卡就不能跑 cuda");
        assert!(caps_cc(true, false, None).cuda_runnable(), "算力未知时乐观放行");
    }

    #[test]
    fn decide_cuda_on_old_card_is_hard_error_with_switch_hint() {
        // GTX 1650 其实是 7.5，这里用 Pascal（6.1）模拟老卡：装了 cuda 版也跑不起来
        let a = assets(Some("cuda"), true, false);
        let err = decide(
            DeviceRequest::Cuda,
            None,
            &a,
            &caps_cc(true, true, Some(61)),
        )
        .unwrap_err();
        assert!(err.contains("算力 6.1"), "{}", err);
        assert!(err.contains("moss_device 改为 vulkan"), "{}", err);
        // 不能再给"去重建 cuda 版"这种死路指引
        assert!(!err.contains("build_moss.ps1"), "{}", err);
    }

    #[test]
    fn decide_cuda_on_turing_still_passes() {
        let eff = parse_info_stderr(STDERR_CUDA);
        let a = assets(Some("cuda"), true, false);
        let d = decide(DeviceRequest::Cuda, eff.as_ref(), &a, &caps_cc(true, true, Some(75))).unwrap();
        assert_eq!(d.backend, "CUDA0");
    }

    #[test]
    fn reject_message_for_old_card_points_to_vulkan_not_rebuild() {
        // 装错后端 + 卡太老：指引优先说"换 vulkan"，而不是"重装 cuda"
        let eff = parse_info_stderr(STDERR_CPU);
        let a = assets(Some("cpu"), false, false);
        let err = decide(DeviceRequest::Cuda, eff.as_ref(), &a, &caps_cc(true, true, Some(61)))
            .unwrap_err();
        assert!(err.contains("moss_device 改为 vulkan"), "{}", err);
    }

    #[test]
    fn recommend_flips_to_vulkan_when_installed_cuda_cannot_run() {
        // 已装 cuda 版 + 老卡 + 本机 Vulkan 可用 ⇒ 推荐 vulkan
        let a = assets(Some("cuda"), true, false);
        let (b, why) = recommend(&a, &caps_cc(true, true, Some(61)));
        assert_eq!(b, "vulkan");
        assert!(why.contains("6.1"), "{}", why);
        // 老卡 + 无 Vulkan ⇒ 只能 cpu
        let (b, _) = recommend(&a, &caps_cc(true, false, Some(61)));
        assert_eq!(b, "cpu");
        // 新卡 + 已装 cuda ⇒ 维持 cuda
        let (b, _) = recommend(&a, &caps_cc(true, true, Some(89)));
        assert_eq!(b, "cuda");
    }

    #[test]
    fn recommend_prefers_vulkan_when_only_cpu_installed_on_old_nvidia() {
        // 只装了 CPU 版 + 有 NVIDIA 老卡 + 有 Vulkan：别推荐跑不起来的 cuda
        let a = assets(Some("cpu"), false, false);
        let (b, why) = recommend(&a, &caps_cc(true, true, Some(61)));
        assert_eq!(b, "vulkan");
        assert!(why.contains("Vulkan"), "{}", why);
    }

    /// 随包发布的真清单（`src-tauri/resources/runtime/manifest.json`）必须可解析，
    /// 且至少声明了 cpu 版——否则下拉只剩 auto，用户装不了任何具体后端。
    #[test]
    fn shipped_manifest_declares_moss_flavors() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/runtime/manifest.json");
        if !path.is_file() {
            return;
        }
        let f = read_manifest_flavors(Some(&path));
        assert!(f.found, "随包 manifest 解析失败：{}", path.display());
        assert!(!f.flavors.is_empty(), "manifest 里没有 id=moss 的 flavor：{:?}", f.flavors);
        assert!(
            f.flavors.contains(&"cpu".to_string()),
            "manifest 缺 cpu flavor（CPU 是兜底后端）：{:?}",
            f.flavors
        );
    }
}
