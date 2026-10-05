//! 融合**能力基准**：干净用例（自撰，非工程产物）判别"跨语言语义对齐"是否真的发生。
//!
//! 与 `OCR_TIMELINE_CLOSURE.md` §三 规划的"替换准确率基准"不同：那个基准要求预校对
//! `.gsa`（素材质量可控），本文件考察的是**模型能力本身**——在无碎片、无重复、每句语义
//! 互不重叠的自撰用例上，看模型是否按内容而非按下标作答。两者互补，勿混用。
//!
//! 判别原理：把语料顺序打成**无不动点排列**（derangement），则"照抄编号"的恒等映射在
//! 每一条上都错（`恒等基线` = 0），内容正确率可直接与随机基线（1/语料条数）比较：
//!   * 正确率 >> 随机 且 >> 恒等基线 ⇒ 在做内容匹配
//!   * 正确率 ≈ 随机 ≈ 恒等基线     ⇒ 只是按下标对齐或纯猜
//! 另设两类对照：① 对齐语料（恒等即正确，验证易例可解）；② 同语言**逐字相同**文本 +
//! 打乱语料顺序（把"翻译难"这一解释隔离掉——连字符串相等都不去利用即证明不做内容匹配）。
//!
//! 打分走**真实管线**（`fuse_pipeline`），比较产出文本与期望文本，因此同时覆盖
//! prompt → 解析 → 合并三段，而不是复刻一份逻辑。
//!
//! 运行（需 runtime LLM 就绪）：
//! ```text
//! cargo test --release --test bench_fusion -- --ignored --nocapture
//! ```
//! 环境变量：`GSA_BENCH_FUSION_REPS`（每例重复次数，默认 1）、`GSA_BENCH_FUSION_CASES`
//! （逗号分隔的用例名子串过滤）、`GSA_BENCH_LLM_MODEL`（临时换模型做能力梯度对照，
//! 不改 runtime/config.json 的产品默认值）、`GSA_BENCH_OUT_DIR`（结果 JSON 落盘目录，
//! 默认 `temp/bench_output`）。
//!
//! 已记录基线（2026-10-02，Qwen2.5-3B-Instruct Q4_K_M，同一 argv）：
//! A1 6/36、A2 3/36、A3 0/90、A4 1/90、C1 36/36、C2 18/39、E2 3/36、E3 36/36，
//! 其中错序用例的恒等值输出占比 33/36（E2）——即"按下标对齐"而非内容匹配。
//! 注意：未固定种子时多次复跑输出可能**逐字相同**，故 REPS>1 不等于独立样本。

use ai_game_subtitle_assistant_lib::ai_runtime::config::RuntimeConfig;
use ai_game_subtitle_assistant_lib::ai_runtime::LlmManager;
use ai_game_subtitle_assistant_lib::fuse::{fuse_pipeline, FuseAsrInput};
use std::path::PathBuf;
use std::time::Instant;

/// 30 组自撰干净对照：(英文 = 转写侧, 中文 = 可靠语料)。
/// 每句完整、语义互不重叠（不共享实体词），故不存在"靠关键词撞上"的捷径。
const PAIRS: [(&str, &str); 30] = [
    ("The seal on the northern gate has weakened.", "北门的封印已经减弱了。"),
    ("I left the supplies by the fountain.", "我把补给放在喷泉旁边了。"),
    ("Do not speak of this to the captain.", "别把这件事告诉队长。"),
    ("The old clock tower stopped at midnight.", "那座旧钟楼在午夜停了。"),
    ("Her fever finally broke this morning.", "她的烧今早终于退了。"),
    ("We need three more lanterns for the festival.", "庆典还需要三盏灯笼。"),
    ("The merchant never returned from the eastern road.", "那个商人再也没从东边那条路回来。"),
    ("Someone has been reading my letters.", "有人在偷看我的信。"),
    ("The river froze earlier than last year.", "河水比去年冻得更早。"),
    ("I can hear footsteps in the cellar.", "我能听见地窖里有脚步声。"),
    ("Please deliver this package before sunset.", "请在日落前把这个包裹送到。"),
    ("The bridge collapsed under the weight of the cart.", "桥被货车的重量压塌了。"),
    ("A stranger asked me about the abandoned mine.", "有个陌生人向我打听那座废弃的矿。"),
    ("The kitchen ran out of salt again.", "厨房的盐又用完了。"),
    ("My sword was not sharpened properly.", "我的剑没有磨好。"),
    ("The choir will practice in the chapel tonight.", "唱诗班今晚在礼拜堂排练。"),
    ("He traded his horse for a broken compass.", "他用马换了一个坏掉的罗盘。"),
    ("The harvest this year was smaller than expected.", "今年的收成比预想的少。"),
    ("Someone carved a warning into the wooden door.", "有人在木门上刻了一句警告。"),
    ("The physician refuses to leave the patient alone.", "医生不肯让病人单独待着。"),
    ("Our water reserves will last two more days.", "我们的水还够撑两天。"),
    ("The beacon on the hill has not been lit.", "山上的烽火没有点起来。"),
    ("I found a torn map inside the old chest.", "我在旧箱子里找到一张撕破的地图。"),
    ("The wolves have moved closer to the village.", "狼群离村子更近了。"),
    ("She has not spoken since the funeral.", "葬礼之后她一直没说话。"),
    ("The blacksmith is still waiting for the iron shipment.", "铁匠还在等那批铁。"),
    ("Nobody remembers who built this tower.", "没人记得这座塔是谁建的。"),
    ("The rain washed away the markings on the road.", "雨水冲掉了路上的标记。"),
    ("He hid the key beneath the third step.", "他把钥匙藏在第三级台阶下面。"),
    ("The council will decide the matter tomorrow.", "议会明天决定这件事。"),
];

/// 12 条固定任意排列（无不动点，构造时校验）
const PERM12: [usize; 12] = [3, 7, 1, 9, 12, 2, 8, 4, 11, 5, 10, 6];

/// 语料中无对应的转写段（期望 ocr_index = 0）
const UNMATCHED_GC: &str = "The innkeeper said the room upstairs is free tonight.";

struct Case {
    name: &'static str,
    /// 每条 GC 来自哪个对照编号（1-based；0 = 语料中无对应）
    gc_pairs: Vec<usize>,
    /// 语料顺序：第 k 条语料来自哪个对照编号（1-based）
    corpus_pairs: Vec<usize>,
    /// true = 语料使用英文原文（同语言逐字相同，用于隔离"翻译难"）
    same_lang: bool,
}

/// 30 条确定性无不动点排列：q(k) = (7k+1) mod 30（7 与 30 互素 ⇒ 双射；无不动点）
fn perm30() -> Vec<usize> {
    (0..30).map(|k| (7 * k + 1) % 30 + 1).collect()
}

fn cases() -> Vec<Case> {
    let natural = |n: usize| (1..=n).collect::<Vec<_>>();
    let mut with_unmatched = Vec::new();
    for p in 1..=12usize {
        with_unmatched.push(p);
        if p == 5 {
            with_unmatched.push(0);
        }
    }
    // 循环移位 1：语料顺序 = [2..=n, 1]（错位 1 且在两端断开）
    let shift12: Vec<usize> = (2..=12).chain(std::iter::once(1)).collect();
    let shift30: Vec<usize> = (2..=30).chain(std::iter::once(1)).collect();
    vec![
        Case { name: "A1-错序12(移位)", gc_pairs: natural(12), corpus_pairs: shift12, same_lang: false },
        Case { name: "A2-错序12(任意)", gc_pairs: natural(12), corpus_pairs: PERM12.to_vec(), same_lang: false },
        Case { name: "A3-错序30(移位)", gc_pairs: natural(30), corpus_pairs: shift30, same_lang: false },
        Case { name: "A4-错序30(任意)", gc_pairs: natural(30), corpus_pairs: perm30(), same_lang: false },
        Case { name: "C1-对齐12(对照)", gc_pairs: natural(12), corpus_pairs: natural(12), same_lang: false },
        Case { name: "C2-对齐12+1无对应", gc_pairs: with_unmatched, corpus_pairs: natural(12), same_lang: false },
        Case { name: "E2-同语言逐字相同(错序)", gc_pairs: natural(12), corpus_pairs: PERM12.to_vec(), same_lang: true },
        Case { name: "E3-同语言逐字相同(对齐)", gc_pairs: natural(12), corpus_pairs: natural(12), same_lang: true },
    ]
}

fn pair_text(pair: usize, same_lang: bool) -> &'static str {
    let (en, cn) = PAIRS[pair - 1];
    if same_lang {
        en
    } else {
        cn
    }
}

/// 期望：GC[i] 应对到语料中"同一对照编号"的位置；无对应（0）则保留转写原文
fn truth(case: &Case) -> Vec<usize> {
    case.gc_pairs
        .iter()
        .map(|p| {
            if *p == 0 {
                0
            } else {
                case.corpus_pairs
                    .iter()
                    .position(|q| q == p)
                    .map(|k| k + 1)
                    .expect("用例构造错误：语料缺少该对照编号")
            }
        })
        .collect()
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

struct Row {
    case: &'static str,
    rep: usize,
    correct: usize,
    total: usize,
    identity_baseline: usize,
    chance_pct: f64,
    pipeline_matched: usize,
    missing_segments: usize,
    failed_batches: usize,
    secs: f64,
}

#[test]
#[ignore]
fn test_fusion_capability_clean_cases() {
    let root = repo_root();
    let runtime_dir = root.join("runtime");
    assert!(
        runtime_dir.join("config.json").is_file(),
        "缺少 runtime/config.json，请先跑 scripts/bootstrap_llm.ps1"
    );
    let mut config = RuntimeConfig::load(&runtime_dir).expect("读取 runtime 配置失败");
    // 能力梯度对照：临时换模型（不改产品默认值）
    if let Ok(m) = std::env::var("GSA_BENCH_LLM_MODEL") {
        config.llm_model = m;
    }
    let model_tag = PathBuf::from(&config.llm_model)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".into());
    let manager = LlmManager::new(config, runtime_dir);
    assert!(
        manager.with_provider(|p| p.is_ready()),
        "LLM 环境未就绪（provider: {}）",
        manager.with_provider(|p| p.name().to_string())
    );

    let reps = env_usize("GSA_BENCH_FUSION_REPS", 1);
    let filter: Vec<String> = std::env::var("GSA_BENCH_FUSION_CASES")
        .map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();

    let mut rows: Vec<Row> = Vec::new();
    println!("\n===== 融合能力基准（干净用例）模型={} reps={} =====", model_tag, reps);
    println!("{:28} {:>4} {:>14} {:>10} {:>8} {:>10} {:>7} {:>7}", "用例", "rep", "内容正确", "恒等基线", "随机%", "管线matched", "未判定", "失败批");
    for case in cases() {
        if !filter.is_empty() && !filter.iter().any(|f| case.name.contains(f.as_str())) {
            continue;
        }
        // 判别器自校验：语料须为排列；错序用例的恒等基线须为 0
        let mut sorted = case.corpus_pairs.clone();
        sorted.sort_unstable();
        let expect_perm: Vec<usize> = (1..=case.corpus_pairs.len()).collect();
        assert_eq!(sorted, expect_perm, "{}：语料不是排列", case.name);
        let truth = truth(&case);
        let idn_base = (0..truth.len()).filter(|&i| truth[i] == i + 1).count();
        // 判别器校验：错序用例（A*、E2）的恒等基线必须为 0；对齐对照（C1/E3）恒等即正确
        if case.name.starts_with('A') || case.name.starts_with("E2") {
            assert_eq!(idn_base, 0, "{}：恒等基线不为 0，判别器失效", case.name);
        }

        let corpus: Vec<String> = case.corpus_pairs.iter().map(|p| pair_text(*p, case.same_lang).to_string()).collect();
        let gc: Vec<String> = case.gc_pairs
            .iter()
            .map(|p| if *p == 0 { UNMATCHED_GC.to_string() } else { PAIRS[*p - 1].0.to_string() })
            .collect();
        let expected: Vec<String> = (0..gc.len())
            .map(|i| if truth[i] == 0 { gc[i].clone() } else { corpus[truth[i] - 1].clone() })
            .collect();
        let inputs: Vec<FuseAsrInput> = gc.iter().enumerate()
            .map(|(i, t)| FuseAsrInput { index: i + 1, start: i as f64, end: i as f64 + 1.0, text: t.clone() })
            .collect();

        for rep in 1..=reps {
            let start = Instant::now();
            let result = fuse_pipeline(&manager, &corpus, &inputs, |_, _| {})
                .unwrap_or_else(|e| panic!("{}：管线失败 {}", case.name, e));
            let secs = start.elapsed().as_secs_f64();
            assert_eq!(result.segments.len(), inputs.len(), "{}：段数不完整", case.name);
            let correct = result.segments.iter().zip(&expected)
                .filter(|(s, want)| s.text.trim() == want.trim())
                .count();
            let row = Row {
                case: case.name,
                rep,
                correct,
                total: inputs.len(),
                identity_baseline: idn_base,
                chance_pct: 100.0 / corpus.len() as f64,
                pipeline_matched: result.stats.matched,
                missing_segments: result.stats.missing_segments,
                failed_batches: result.stats.failed_batches,
                secs,
            };
            println!("{:28} {:>4} {:>14} {:>10} {:>9.1} {:>10} {:>8} {:>7}",
                row.case, row.rep, format!("{}/{}", row.correct, row.total),
                row.identity_baseline, row.chance_pct, row.pipeline_matched,
                row.missing_segments, row.failed_batches);
            rows.push(row);
        }
        println!("   —— {} 用时 {:.1}s", case.name, rows.last().map(|r| r.secs).unwrap_or(0.0));
    }

    // 落盘：便于 3B / 7B / 更大模型的同构建 A/B
    let out_dir = std::env::var("GSA_BENCH_OUT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root.join("temp").join("bench_output"));
    let _ = std::fs::create_dir_all(&out_dir);
    let json = format!(
        "{{\n \"model\": \"{}\",\n \"reps\": {},\n \"rows\": [\n{}\n ]\n}}\n",
        model_tag,
        reps,
        rows.iter()
            .map(|r| format!(
                "  {{\"case\":\"{}\",\"rep\":{},\"correct\":{},\"total\":{},\"identity_baseline\":{},\"chance_pct\":{:.2},\"pipeline_matched\":{},\"missing_segments\":{},\"failed_batches\":{},\"secs\":{:.2}}}",
                r.case, r.rep, r.correct, r.total, r.identity_baseline, r.chance_pct,
                r.pipeline_matched, r.missing_segments, r.failed_batches, r.secs))
            .collect::<Vec<_>>()
            .join(",\n")
    );
    let path = out_dir.join(format!("fusion_capability_{}.json", model_tag));
    if let Err(e) = std::fs::write(&path, json) {
        eprintln!("[bench_fusion] 结果落盘失败 {}: {}", path.display(), e);
    } else {
        println!("\n结果已落盘：{}", path.display());
    }

    // 判读提示：错序用例的正确率若贴近随机基线、且恒等基线为 0，即"未做内容匹配"
    let deranged: Vec<&Row> = rows.iter().filter(|r| r.case.starts_with('A') || r.case.starts_with("E2")).collect();
    if !deranged.is_empty() {
        let ok: usize = deranged.iter().map(|r| r.correct).sum();
        let tot: usize = deranged.iter().map(|r| r.total).sum();
        println!("错序用例合计 {}/{}（随机期望 {:.0}）", ok, tot,
            deranged.iter().map(|r| r.total as f64 * r.chance_pct / 100.0).sum::<f64>());
    }
}
