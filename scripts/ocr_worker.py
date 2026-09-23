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
        **extra,
    )


def _extract_lines(res):
    """从 OCR 结果里逐行产出 (text, confidence)。

    paddleocr 3.x 的 `res` 可能是：
      - dict: {"rec_texts": [...], "rec_scores": [...], ...}
      - list: [{"text": ..., "confidence": ...}, ...] 或 [str, ...]
    """
    if isinstance(res, dict):
        for t, c in zip(res.get("rec_texts", []), res.get("rec_scores", [])):
            yield t or "", float(c or 0.0)
    elif isinstance(res, list):
        for it in res:
            if isinstance(it, dict):
                text = it.get("text") or ""
                conf = float(it.get("confidence") or 0.0)
            else:
                text = str(it)
                conf = 0.0
            yield text, conf


def _extract_result(result):
    """从单条 predict 结果提取 (text, confidence)。

    后处理链：低置信度行过滤（conf < CONF_THRESHOLD 丢弃）→
    白名单/行级清理（_clean_text）。整体置信度取剩余行最低值。
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

    texts = []
    confs = []
    for text, conf in _extract_lines(res):
        text = (text or "").strip()
        if text and conf >= CONF_THRESHOLD:
            texts.append(text)
            confs.append(conf)
    if not texts:
        return "", 0.0
    cleaned = _clean_text("\n".join(texts))
    return cleaned, min(confs) if cleaned else 0.0


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
