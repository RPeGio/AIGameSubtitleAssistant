# -*- coding: utf-8 -*-
"""OCR Worker：Rust 侧常驻子进程，通过 JSON lines over stdio 通信。

协议（stdin 每行一个请求，stdout 每行一个响应，UTF-8）：
    请求: {"id":1,"images":["<base64 JPEG>","<base64 JPEG>"]}
          {"id":2,"cmd":"ping"}
          {"id":3,"cmd":"shutdown"}
    响应: {"id":1,"ok":true,"results":[{"text":"..","confidence":0.98},...]}
          {"id":1,"ok":false,"error":".."}
          {"id":2,"ok":true,"cmd":"pong"}

图像以 base64 编码的 JPEG 字节传输，worker 在内存中解码为 ndarray 后交给
predict——全程不产生临时文件，也天然规避 cv2 读图的非 ASCII 路径问题。
模型只在首次需要时加载一次（内存常驻）。Rust 侧注入：
  - PADDLE_PDX_CACHE_HOME：模型缓存目录（runtime/models/paddleocr）
  - GSA_OCR_MODEL：模型档位 "mobile"（默认，快）| "server"（慢，更准）
  - GSA_OCR_DEVICE：推理设备 "cpu" | "gpu:0"（见 _resolve_device；空 = 交给 paddlex 自动选）
  - GSA_OCR_TF32：设 "1" 才允许 TF32（默认关闭，见下方 TF32 段）
批量请求用一次 `ocr.predict(images列表)` 完成（真批处理）。
"""

import base64
import io
import json
import os
import sys

LANG = "ch"

# ─── TF32：默认关闭，换取与 CPU 逐字节一致的产出 ──────────────
# Ada（sm_89）及以上的 cuBLAS/cuDNN 默认用 TF32 张量核做 FP32 矩阵乘（尾数 23→10 位），
# 会让形近字形（「」/】/] 之类）的 argmax 在 CPU/GPU 间翻转。实测 bench_corpus 三案例
# 共 4 处单字符标点差异，评分/CER 不受影响（评分口径去标点）但产出 SRT 不再逐字节一致。
# 关掉后产出与 CPU 完全一致，代价约 3.8%（端到端 174.3s → 180.9s）。
# 需要那 3.8% 时设 GSA_OCR_TF32=1。用 setdefault：用户已显式设 NVIDIA_TF32_OVERRIDE 时尊重之。
# 必须在 import paddle 之前写入——CUDA 上下文建立后再设无效。
if os.environ.get("GSA_OCR_TF32", "").strip() != "1":
    os.environ.setdefault("NVIDIA_TF32_OVERRIDE", "0")

# base64 → ndarray 解码依赖（均为 paddleocr 的传递依赖，runtime/deps 内自带）
import cv2
import numpy as np

# 模型档位 → 检测/识别模型名
MODEL_MAP = {
    "mobile": ("PP-OCRv5_mobile_det", "PP-OCRv5_mobile_rec"),
    "server": ("PP-OCRv5_server_det", "PP-OCRv5_server_rec"),
}

# ─── 确定性后处理：字符白名单 + 低置信度行过滤 ──────────────
# 只做机械清理，不尝试补字/改写 —— 保证字幕原文不被改动。
# 白名单范围：
#   - 0x20-0x7E    半角可打印 ASCII（字母/数字/英文标点）
#   - 0x3000-0x303F CJK 标点（、。《》「」等，含全角空格）
#   - 0x3040-0x30FF 日文假名
#   - 0x3400-0x4DBF/0x4E00-0x9FFF CJK 统一表意文字（含扩展 A）
#   - 0xFF00-0xFFEF 全角字母数字与全角标点
_ALLOWED_RANGES = (
    (0x20, 0x7E),
    (0x3000, 0x303F),
    (0x3040, 0x30FF),
    (0x3400, 0x4DBF),
    (0x4E00, 0x9FFF),
    (0xFF00, 0xFFEF),
)

# 范围外但字幕常见、需额外保留的字符：
# · 间隔号（"卡侬·桑娜妲"）、—/–/― 破折号、弯引号、… 省略号、※、← →
_EXTRA_KEEP = frozenset(
    chr(cp)
    for cp in (
        0x00B7, 0x2013, 0x2014, 0x2015, 0x2018, 0x2019,
        0x201C, 0x201D, 0x2026, 0x203B, 0x2190, 0x2192,
    )
)

# 单行识别置信度下限：低于此值的行视为噪音丢弃（GSA_OCR_CONF_THRESHOLD 覆盖）
CONF_THRESHOLD = float(os.environ.get("GSA_OCR_CONF_THRESHOLD", "0.5"))


def _char_allowed(ch):
    if ch in _EXTRA_KEEP:
        return True
    cp = ord(ch)
    return any(lo <= cp <= hi for lo, hi in _ALLOWED_RANGES)


def _clean_text(text):
    """白名单过滤 + 行级清理：去空行、行首尾空白、连续重复行。"""
    lines = []
    for raw in text.split("\n"):
        line = raw.strip()
        if not line:
            continue
        kept = [ch for ch in line if _char_allowed(ch)]
        line = "".join(kept).strip()
        if line and (not lines or line != lines[-1]):
            lines.append(line)
    return "\n".join(lines)


def _resolve_device():
    """把 GSA_OCR_DEVICE 解析成可用的设备串；空串表示不指定（交给 paddlex 自动选）。

    空 = 不传 device：paddlex 的 get_default_device() 会在 paddle 编译了 CUDA 且有
    设备时自动选 gpu:0，否则 cpu —— 未启用 GPU 时行为与改动前完全一致。

    显式要 gpu 但 CUDA 不可用时回退 cpu：Rust 侧只在 deps_gpu 存在时才把它挂到
    PYTHONPATH 首位，所以"配了 gpu:0 但没跑 bootstrap_ocr_gpu.ps1"是常见误配；
    回退 + 一行 stderr 提示，好过整轮 OCR 直接报错。
    """
    want = os.environ.get("GSA_OCR_DEVICE", "").strip().lower()
    if not want:
        return ""
    device_type = want.split(":")[0]
    if device_type != "gpu":
        return want  # cpu 原样；npu/xpu 等交给 paddle 自己报错，不臆测
    try:
        import paddle

        if paddle.device.is_compiled_with_cuda() and paddle.device.cuda.device_count() > 0:
            return want
    except Exception as e:
        sys.stderr.write(f"[ocr_worker] 探测 CUDA 失败（{e}），回退 cpu\n")
        return "cpu"
    sys.stderr.write(
        f"[ocr_worker] 请求 {want} 但当前 paddle 无可用 CUDA，回退 cpu；"
        "如需 GPU 请先运行 scripts/bootstrap_ocr_gpu.ps1\n"
    )
    return "cpu"


# ─── det 分批：把"逐张跑 det"改成"一次 16 张" ──────────────
# paddlex 的各子模型**总是**按自己的 batch_sampler 分块再逐块 process
# （base_predictor.py:337 `for batch_data in batches: process(batch_data)`），而
# paddlex 的 OCR.yaml 里 TextDetection 段没有 batch_size → 默认 1，也就是管线哪怕
# 一次拿到 16 张，det 仍然逐张推理。实测（RTX 4060 + PP-OCRv5_mobile，886×124 语料帧）
# 把 det 批设为 16 后 36.0 → 22.2 ms/帧（1.62×），且 16 已是平台（32/64 无进一步收益）。
# 取值与 Rust 侧默认 IPC 批（OcrRunParams.batch_size = 16）对齐：det 实际批 =
# min(IPC 批, 本值)，IPC 批更小时只是填不满，不会出错。
# rec 侧不受影响：它逐输入图像调用（pipeline.py:446），子批上限就是单帧行数。
DET_BATCH = 16


def _paddlex_config():
    """取官方 OCR 管线配置并把 det 批大小改成 DET_BATCH。

    只能走 paddlex_config：paddleocr 的 PaddleOCR 包装层没有暴露
    text_detection_batch_size（只有 ..._recognition_/_textline_orientation_ 两个）。
    注意该参数会**整体替换**基线配置（_pipelines/base.py:95 把传入对象直接当基线），
    所以必须先 load_pipeline_config("OCR") 取回官方配置再改，不能只传局部 dict，
    否则 SubModules/PreProcess 等会全部缺失。
    """
    from paddlex.inference import load_pipeline_config

    cfg = load_pipeline_config("OCR")
    cfg["batch_size"] = DET_BATCH
    cfg["SubModules"]["TextDetection"]["batch_size"] = DET_BATCH
    return cfg


def make_ocr():
    from paddleocr import PaddleOCR

    model = os.environ.get("GSA_OCR_MODEL", "mobile").strip().lower()
    det, rec = MODEL_MAP.get(model, MODEL_MAP["mobile"])
    device = _resolve_device()
    # 记录实际生效的设备：GPU 基准对比时靠这一行确认真的跑在 GPU 上
    sys.stderr.write(f"[ocr_worker] paddle 设备: {device or 'auto'}（GSA_OCR_MODEL={model}）\n")
    extra = {"device": device} if device else {}
    return PaddleOCR(
        lang=LANG,
        text_detection_model_name=det,
        text_recognition_model_name=rec,
        use_doc_orientation_classify=False,
        use_doc_unwarping=False,
        use_textline_orientation=False,
        paddlex_config=_paddlex_config(),
        **extra,
    )


def _extract_items(res):
    """逐框产出 (text, confidence, box)——box 为该框的 xyxy（无几何字段时为 None）。

    dict 路径按 rec_texts/rec_scores/rec_boxes 下标对齐；list 兼容路径无 box。
    """
    if isinstance(res, dict):
        boxes = res.get("rec_boxes")
        n = len(res.get("rec_texts", []))
        aligned = boxes is not None and len(boxes) == n
        for i, (t, c) in enumerate(
            zip(res.get("rec_texts", []), res.get("rec_scores", []))
        ):
            box = boxes[i] if aligned else None
            yield t or "", float(c or 0.0), (box if aligned else None)
    elif isinstance(res, list):
        for it in res:
            if isinstance(it, dict):
                text = it.get("text") or ""
                conf = float(it.get("confidence") or 0.0)
            else:
                text = str(it)
                conf = 0.0
            yield text, conf, None


def _is_latin_alnum(ch):
    return ch.isascii() and ch.isalnum()


def _join_row(items):
    """同一物理行内按 x 升序拼接。

    两侧都是 ASCII 字母/数字才补空格（`I`+`have`→`I have`，修复 det 行内
    碎片拼接丢空格——moon 实测 `no objections, Ihave nothing...`）；
    中文/标点直接相连（`上`+`吧`→`上吧`，中文无空格）。
    """
    acc = ""
    for _, t in sorted(items, key=lambda it: it[0]):
        t = t.strip()
        if not t:
            continue
        if acc:
            sep = " " if _is_latin_alnum(acc[-1]) and _is_latin_alnum(t[0]) else ""
            acc += sep + t
        else:
            acc = t
    return acc


def _lines_geometric(items):
    """几何行重排（O2 修复，2026-10-13）。

    根因：det 返回框序**不保证**"行内左→右 / 行间自上而下"——vesna 实测同一
    物理行的两框在 rec_texts 里被另一行的空框隔开（clip_0 frame_00039：
    rec=VC/空框/a，几何上 VC 与 a 同行）、单行显示被 det 拆成 2~7 个碎片框
    （台账例 `So, I hope\\nhad\\nnight……` 语序错乱）。先前 `_extract_result`
    直接按 rec 顺序 `\\n` 连接 ⇒ 嵌字侧 29/74 段含换行且部分语序错乱。

    修复：对已通过置信度过滤的 (text, conf, box) 三元组按几何重排——
    **物理行分组**：相邻框（按 y 中心排序）的**竖向重叠** > 0.4×较矮框高
    才算同一物理行，否则断行。行内按 x 中心升序拼接（`_join_row`）。
    **行间仍以 \\n 分隔**——D9/D11/D12 合并层依赖"姓名框+对话行"的多行结构，
    禁止拍平（merge.rs 的 is_line_prefix/is_stable_line_boundary_cut/comparable_line）。

    判据标定（2026-10-13 四帧实测，帧存 temp/t3_dump3_out.txt）：
    - 真同行碎片：y 重叠 68~75px（≈整行高，如 ".Fabricating"/"import log…"、
      "And using…" 与 "gemstones?"——后者把台账 O2 例句的伪 2 行还原为 1 行）；
    - 物理相邻行（姓名框 58→66、对话两行 99→101）：重叠 0~5px（<0.15×行高）。
    - **教训**：首版判据"竖向空隙 > 0.5×较矮框高即断行"方向反了——行距 2~9px
      的相邻行被判同行，整帧 3 行连成一句，嵌字四卡崩盘（moon 71.4→-5.1、
      pierro 71.7→-13.1、glupov 71.5→47.2，temp/bench_output/t3_hardsub.log）。
    - 次版教训：先按 conf 过滤再对齐 `rec_boxes` 会**下标错位**（rec_boxes 覆盖
      全部 det 框，被滤框会让 len 对不上、整帧回落旧序——frame_00217 因此
      仍出 2 行）。**必须带着框索引过滤**：本函数收到的是已对齐的三元组。

    任一框缺失几何（兼容路径/形状异常）→ 返回 None，回落 rec 顺序。
    """
    if not items or any(it[2] is None for it in items):
        return None
    try:
        import numpy as _np

        b = _np.asarray([it[2] for it in items], dtype=float)
    except Exception:
        return None
    if b.ndim != 2 or b.shape[1] != 4:
        return None
    yc = (b[:, 1] + b[:, 3]) / 2.0
    order = sorted(range(len(items)), key=lambda i: yc[i])
    rows = []
    cur = [order[0]]
    for i in order[1:]:
        p = cur[-1]
        overlap = min(b[i][3], b[p][3]) - max(b[i][1], b[p][1])
        min_h = min(b[i][3] - b[i][1], b[p][3] - b[p][1])
        if overlap > 0.4 * min_h:
            cur.append(i)
        else:
            rows.append(cur)
            cur = [i]
    rows.append(cur)
    out = []
    for row in rows:
        rowitems = [((b[i][0] + b[i][2]) / 2.0, items[i][0]) for i in row]
        out.append(_join_row(rowitems))
    out = [t for t in out if t]
    if not out:
        return None
    return "\n".join(out)


def _extract_result(result):
    """从单条 predict 结果提取 (text, confidence)。

    后处理链：低置信度框过滤（conf < CONF_THRESHOLD 丢弃，**带框索引**）→
    几何行重排（`_lines_geometric`，有 rec_boxes 时；否则沿用 rec 顺序）→
    白名单/行级清理（_clean_text）。整体置信度取剩余框最低值。
    """
    if result is None:
        return "", 0.0
    res = None
    try:
        res = result.json.get("res")
    except Exception:
        res = None
    if res is None:
        try:
            res = result.res
        except Exception:
            res = None
    if res is None:
        return "", 0.0

    items = []  # (text, confidence, box) 过滤后保留
    for text, conf, box in _extract_items(res):
        text = (text or "").strip()
        if text and conf >= CONF_THRESHOLD:
            items.append((text, conf, box))
    if not items:
        return "", 0.0
    joined = _lines_geometric(items)
    if joined is None:
        joined = "\n".join(t for t, _, _ in items)
    cleaned = _clean_text(joined)
    return cleaned, min(c for _, c, _ in items) if cleaned else 0.0


def respond(stdout, rid, **payload):
    payload["id"] = rid
    stdout.write(json.dumps(payload, ensure_ascii=False) + "\n")
    stdout.flush()


def main():
    stdin = io.TextIOWrapper(sys.stdin.buffer, encoding="utf-8", errors="replace")
    stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
    ocr = None

    for line in stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except Exception as e:
            respond(stdout, None, ok=False, error=f"JSON 解析失败: {e}")
            continue

        rid = req.get("id")
        try:
            cmd = req.get("cmd")
            if cmd == "shutdown":
                respond(stdout, rid, ok=True, cmd="ack")
                break
            if ocr is None:
                ocr = make_ocr()
            if cmd == "ping":
                respond(stdout, rid, ok=True, cmd="pong")
            elif "images" in req:
                images = req["images"]
                # base64 JPEG → 内存解码为 ndarray（不碰磁盘）
                inputs = []
                for item in images:
                    raw = base64.b64decode(item)
                    img = cv2.imdecode(np.frombuffer(raw, np.uint8), cv2.IMREAD_COLOR)
                    if img is None:
                        raise ValueError("图像解码失败（base64 数据无效）")
                    inputs.append(img)
                # 真批处理：一次 predict 列表
                results = ocr.predict(inputs) if inputs else []
                out = []
                if isinstance(results, list):
                    for r in results:
                        text, conf = _extract_result(r)
                        out.append({"text": text, "confidence": conf})
                else:
                    # 极少数情况返回单个结果
                    text, conf = _extract_result(results)
                    out.append({"text": text, "confidence": conf})
                respond(stdout, rid, ok=True, results=out)
            else:
                respond(stdout, rid, ok=False, error=f"未知请求: {req}")
        except Exception as e:
            respond(stdout, rid, ok=False, error=str(e))


if __name__ == "__main__":
    main()
