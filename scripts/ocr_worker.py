# -*- coding: utf-8 -*-
"""OCR Worker：Rust 侧常驻子进程，通过 JSON lines over stdio 通信。

协议（stdin 每行一个请求，stdout 每行一个响应，UTF-8）：
    请求: {"id":1,"images":["/path/a.jpg","/path/b.jpg"]}
          {"id":2,"cmd":"ping"}
          {"id":3,"cmd":"shutdown"}
    响应: {"id":1,"ok":true,"results":[{"text":"..","confidence":0.98},...]}
          {"id":1,"ok":false,"error":".."}
          {"id":2,"ok":true,"cmd":"pong"}

模型只在首次需要时加载一次（内存常驻）。Rust 侧注入：
  - PADDLE_PDX_CACHE_HOME：模型缓存目录（runtime/models/paddleocr）
  - GSA_OCR_MODEL：模型档位 "mobile"（默认，快）| "server"（慢，更准）
批量请求用一次 `ocr.predict(images列表)` 完成（真批处理）。
"""

import io
import json
import os
import sys

LANG = "ch"

# 模型档位 → 检测/识别模型名
MODEL_MAP = {
    "mobile": ("PP-OCRv5_mobile_det", "PP-OCRv5_mobile_rec"),
    "server": ("PP-OCRv5_server_det", "PP-OCRv5_server_rec"),
}


def make_ocr():
    from paddleocr import PaddleOCR

    model = os.environ.get("GSA_OCR_MODEL", "mobile").strip().lower()
    det, rec = MODEL_MAP.get(model, MODEL_MAP["mobile"])
    return PaddleOCR(
        lang=LANG,
        text_detection_model_name=det,
        text_recognition_model_name=rec,
        use_doc_orientation_classify=False,
        use_doc_unwarping=False,
        use_textline_orientation=False,
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
    """从单条 predict 结果提取 (text, confidence)。"""
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
        if text:
            texts.append(text)
            confs.append(conf)
    return "\n".join(texts), min(confs) if confs else 0.0


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
                # 真批处理：一次 predict 列表
                results = ocr.predict(images) if images else []
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
