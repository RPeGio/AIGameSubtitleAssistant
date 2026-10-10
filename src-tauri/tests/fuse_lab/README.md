# fuse_lab —— 融合对齐实验台

跨语言对齐路线的**判别器与离线验证**，供"融合管线重构"（feat 分支）直接复用。
不是产品代码，也不进 `cargo test` 自动流程（需 runtime 运行时）。

## 为什么需要它

`benchmark/FUSE_PIPELINE_DEFECTS.md` F13 证明了现行 LLM 融合**没有在做跨语言语义对齐**
（按下标对齐，错序判别器上=随机）；`benchmark/FUSE_VECTOR_RECALL_VALIDATION.md` 证明
**多语言向量召回可以替代它**（moon 15/15、glupov 22/22，错序判别器 100%）。

这套实验台就是那两份结论的**可复现工具**，也是重构的判据来源。
最新端到端数字与阈值/判据标定（四案例 **226/236 计分段**、「该不配」掩码判据、告警重标）见
`benchmark/FUSE_THRESHOLD_CALIBRATION.md` §7.9；该路线落地产品（T4）计划与前端「融合后置信度审计表」**同轮做**，见 `GameSubtitleAssistant_Plan.md` §6.3。

## 判别器原理（关键）

把语料顺序打成**无不动点排列**（derangement），使"照抄编号"的恒等映射在**每一条上都错**
⇒ 内容正确率可直接与随机基线（1/语料条数）比较：

| 观察 | 结论 |
|---|---|
| 正确率 ≈ 随机 ≈ 恒等基线 | 只是按下标对齐或纯猜 |
| 正确率 >> 随机（恒等基线=0） | 确实在做内容匹配 |

另设三类对照：① 对齐语料（恒等=正确，验证易例可解）；② 插入一条无语料对应的行（0 路径）；
③ **同语言逐字相同** + 打乱语料（隔离"翻译难"这一解释——连字符串相等都不利用即证明不做内容匹配）。

## 三个入口

```powershell
# 依赖与模型（一次性；模型 113MB，CPU 推理，不占 GPU）
powershell -ExecutionPolicy Bypass -File scripts/bootstrap_embed.ps1 -HfMirror https://hf-mirror.com

$env:PYTHONPATH = "<repo>\runtime\deps_embed"

# ① LLM 判别器：模型是否具备跨语言语义对齐（走真实 llama-cli，同 run_complete argv）
python src-tauri/tests/fuse_lab/bench_fusion_cases.py [reps]

# ② 向量召回判别器：recall@k / MRR（判定"召回 + 复核"是否可行）
python src-tauri/tests/fuse_lab/bench_fusion_recall.py

# ③ 离线验证流水线：预校对工程 → 召回 + 单调 DP → SRT + 逐段判定表
python src-tauri/tests/fuse_lab/fuse_offline_validate.py [project.gsa ...]
```

环境变量：`GSA_EMBED_MODEL`、`GSA_EMBED_ONNX`（换量化档）、`GSA_BENCH_LLM_MODEL`（换 LLM）、
`GSA_BENCH_OUT_DIR`（产物目录，默认 `temp/bench_output`）。

## 已记录基线（2026-10-05）

| 判别器 | Qwen2.5-3B | multilingual-e5-small |
|---|---|---|
| 错序用例（恒等基线=0） | **7/96**（随机期望 5 → 无对齐能力） | **recall@1 96/96、MRR 1.000** |
| 真实工程（moon / glupov） | 恒等映射（错） | **15/15、22/22**，产物与语料逐字一致 |

## 两条不可动摇的实现约定

1. **表头（名字行/头衔行/名牌）剥离只用于模型输入**（编码/检索）。
   表头在段间完全同形会淹没正文语义——glupov 带表头时召回 3/22，剥离后 22/22。
   **产物文本必须是语料原文逐字**（含表头），`split_header` 只作用于编码路径。
2. **单调性必须真正强制**：语料与转写同序 ⇒ 匹配下标严格递增。
   曾因"无对应"分支沿用 `argmax` 前驱导致下标回退/回绕（表现为整体错位一位），
   现由 `monotonic_align` 的 `k > j` 转移保证。

## 文件

| 文件 | 作用 |
|---|---|
| `__init__.py` | 共用底座：用例定义、向量编码、单调 DP、表头处理、`.gsa` 读取、LLM 直调 |
| `bench_fusion_cases.py` | ① LLM 判别器 |
| `bench_fusion_recall.py` | ② 向量召回判别器 |
| `fuse_offline_validate.py` | ③ 离线验证流水线（产物：`temp/bench_output/fuse_vec_*.{srt,json}`） |
