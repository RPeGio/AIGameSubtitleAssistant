# 基准素材（benchmark_examples）

## 已入库（小文件，clone 即可用）

| 文件 | 用途 |
|---|---|
| `moon_sisters.gsa` | 案例 moon 工程（有配音，ASR 15 段）|
| `glupov.gsa` | 案例 glupov 工程（无配音，嵌字 22 段）|
| `pierro_questions.gsa` | 案例 pierro 工程（有配音，嵌字 120 段）|
| `vesna_trailer.gsa` | 案例 vesna 工程（主播 PV reaction，嵌字 82 段）|
| `quality_bench_test(voiced)_5min_reference.txt` | moon 参考文本 |
| `quality_bench_test(non-voiced)_11min_reference.txt` | glupov 参考文本 |
| `quality_bench_test(voiced)_48min_reference.txt` | pierro 参考文本 |
| `pv_reaction_vesna(voiced)_12min_reference.txt` | vesna 参考文本 |

**这些足够复现「融合」基准**——融合侧（`src-tauri/tests/fuse_lab/`、`scripts/fuse_*.py`）
只读 `.gsa` 的 `tracks` 与参考文本，**不读视频**：

```
$env:PYTHONPATH="<repo>\runtime\deps_embed"
& runtime\python\python.exe scripts/fuse_truth.py      # 真值
& runtime\python\python.exe scripts/fuse_calib.py      # 标定与四案例分数
& runtime\python\python.exe scripts/fuse_align_srt.py  # 融合产物 SRT
```

`benchmark/timebase/*_timebase.json` 也已入库，它带 clip/参考文件的 SHA256 守卫。

## 未入库（需另行获取）

**OCR 基准所需的 mp4 素材**（语料片 + 交付片，合计约 2.9 GB）：

| 文件 | 体积 |
|---|---|
| `quality_bench_test(voiced)_48min.mp4` | 1.4 GB |
| `quality_bench_test_corpus(voiced)_48min.mp4` | 409 MB |
| `pv_reaction_vesna(voiced)_12min.mp4` | 407 MB |
| `quality_bench_test(non-voiced)_11min.mp4` | 373 MB |
| `quality_bench_test(voiced)_5min.mp4` | 123 MB |
| `pv_reaction_vesna_corpus(voiced)_12min.mp4` | 85 MB |
| `quality_bench_test_corpus(voiced)_5min.mp4` | 43 MB |
| `quality_bench_test_corpus(non-voiced)_11min.mp4` | 6 MB |

体积原因**不入 git**（GitHub LFS 免费额度 1 GiB 也不够）。
获取方式：**GitHub Release 资产 + 抓取脚本**（计划中，脚本落地后在此补下载命令与 SHA256 校验说明）。

## 已知事项

- **`.gsa` 已清洗掉本机绝对路径**：顶层 `path` / `video` / `source_video` 置为**空串**
  （`tracks` 内的 `video` 是 `source` / `clip` **枚举**，未改动）。融合侧只读 `tracks`，
  故**基准分数逐位不受影响**（实测清洗前后四案例 **226/236** 一致）；在 GUI 中打开工程时
  视频路径为空，需手动重新指定（空 `source_video` 是受支持的合法状态，
  见 `src-tauri/tests/common/mod.rs::test_project_legacy_json_defaults_source_video_corpus`）。
- 每个案例用哪个视频，见 `src-tauri/tests/common/mod.rs` 的 `CaseCfg.clip_video` /
  `corpus_video`，以及 `scripts/fuse_truth.py` 的对应表。
- `pierro_fixed.gsa` 是历史中间产物，无代码/文档引用，**未入库**（`.gitignore` 已显式排除）。
