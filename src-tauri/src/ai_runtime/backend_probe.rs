// ─── MOSS（ggml）后端探测与判定 ───────────────────────────
// 实现设计文档 docs/engines_vulkan_design.md §3.2（三级探测）/ §3.4（设备矩阵）：
//   L0 机器能力：nvidia-smi / vulkaninfo 探机器"能跑什么"
//   L1 资产一致性：.backend 标记 + ggml-{cuda,vulkan}.dll 名，探"装了什么"
//   L2 实证：`info <model>` 的 stderr `[mt I] backend:` 行，唯一可信判据
// 语义：显式请求（cpu/cuda/vulkan）不可用 = 硬报错，绝不自动降级
//（ggml 静默降级时 exit code 仍为 0，L2 行才是真实后端）；只有 auto（空）
// 允许 ggml 自选最优。设备名映射：CUDA0 / Vulkan0 / CPU（显式 vulkan 裸值
// 会选中枚举序第一块 GPU，双 GPU 机器上常是核显，见 §2.4 实测）。

use std::path::Path;
use std::process::Command;

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

/// L0：机器能力探测结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MachineCaps {
    pub cuda: bool,
    pub vulkan: bool,
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
    v.trim_start_matches(|c: char| c == '=' || c == ' ').trim()
}

// ─── L0 / L1 探测（有子进程/IO，成本：nvidia-smi ~50ms，vulkaninfo ~200ms）──

pub fn probe_machine() -> MachineCaps {
    let cuda = Command::new("nvidia-smi")
        .arg("-L")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    // Vulkan：运行时 loader（驱动提供）+ vulkaninfo 能枚举到 GPU
    let vulkan = vulkan_loader_present() && vulkaninfo_gpus_nonempty();
    MachineCaps { cuda, vulkan }
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
        DeviceRequest::Cuda => verify_explicit("cuda", "CUDA", eff, assets, caps),
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
    // info 没给 backend 行（老版本二进制）：退回资产推断，能对上就放行
    let marker_ok = assets
        .marker
        .as_deref()
        .map(|m| m.eq_ignore_ascii_case(req_raw))
        .unwrap_or(false);
    if marker_ok || (expect == "CUDA" && assets.has_cuda_dll)
        || (expect == "Vulkan" && assets.has_vulkan_dll)
    {
        Ok(Decision {
            backend: req_raw.to_string(),
            reason: format!("info 未实证 backend 行，按已装资产（.backend={}）放行", req_raw),
            source: "l1",
        })
    } else {
        Err(reject_message(req_raw, "（未实证，info 无 backend 行且资产不符）", caps))
    }
}

/// 拒绝文案：按 L0 给"装对应后端"或"改回 auto"的指引（§3.4）
fn reject_message(req_raw: &str, actual: &str, caps: &MachineCaps) -> String {
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
        MachineCaps { cuda, vulkan }
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
}
