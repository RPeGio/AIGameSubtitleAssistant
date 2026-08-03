# -*- coding: utf-8 -*-
"""OCR Worker：Rust 侧常驻子进程，通过 JSON lines over stdio 通信。

协议（stdin 每行一个请求，stdout 每行一个响应，UTF-8）：
    请求: {"id":1,"images":["/path/a.jpg","/path/b.jpg"]}
          {"id":2,"cmd":"ping"}
          {"id":3,"cmd":"shutdown"}
    响应: {"id":1,"ok":true,"results":[{"text":"..","confidence":0.98},...]}
          {"id":1,"ok":false,"error":".."}
          {"id":2,"ok":true,"cmd":"pong"}

模型只在首次需要时加载一次（内存常驻）。Rust 侧注入
PADDLE_PDX_CACHE_HOME（paddleocr 3.x 基于 paddlex），使模型下载/缓存
落在 runtime/models/paddleocr。
"""

import io
import json
import sys

LANG = "ch"


def make_ocr():
    from paddleocr import PaddleOCR

    return PaddleOCR(
        lang=LANG,
        use_doc_orientation_classify=False,
        use_doc_unwarping=False,
        use_textline_orientation=False,
    )


def recognize(ocr, image_path):
    """识别单张图，返回 (text, confidence)。
    区域多行文本用换行拼接，置信度取各行最小值。
    """
    result = ocr.predict(image_path)
    if not result:
        return "", 0.0

    items = []
    try:
        items = result[0].json.get("res") or []
    except Exception:
        items = []
    if not items:
        try:
            res = result[0].res
            items = [
                {"text": t, "confidence": c}
                for t, c in zip(res.get("rec_texts", []), res.get("rec_scores", []))
            ]
        except Exception:
            items = []

    texts = []
    confs = []
    for it in items:
        text = (it.get("text") or "").strip()
        conf = float(it.get("confidence") or 0.0)
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
                results = []
                for img in req["images"]:
                    text, conf = recognize(ocr, img)
                    results.append({"text": text, "confidence": conf})
                respond(stdout, rid, ok=True, results=results)
            else:
                respond(stdout, rid, ok=False, error=f"未知请求: {req}")
        except Exception as e:
            respond(stdout, rid, ok=False, error=str(e))


if __name__ == "__main__":
    main()
