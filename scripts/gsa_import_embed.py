#!/usr/bin/env python3
"""把 OCR 产物逆解析回 .gsa 工程的 embed_ocr 轨道（省去在 GUI 内重跑嵌字 OCR）。

用途：`bench_pierro_embed_ocr` 已产出 temp/bench_output/pierro_embed_ocr.json，
把它写成工程内的 `embed_ocr` 产物轨，用户即可直接在 GUI 里做预校对。

严格对齐产品行为（src/stores/project.ts）：
  · ensureEmbedOcrTrack：轨道对象字段与 `tracks.push` 位置（追加到末尾）一致
  · embedOcrToEvent  ：事件 {id,start,end,text,confidence}（Rust 侧 tag=type 在前）
  · writeEmbedOcrSegments：`track.events` 按 start 升序**整体替换**（重跑不叠加）
  · generateId()      ：base36(ms) + 6 位随机 base36
文件格式与 Rust 保存一致：首行魔数 `GSA-PROJECT v1` + 2 空格缩进 JSON + LF + 无 BOM + 末尾换行。

安全措施：
  · 写入前备份原文件为 <name>.bak-<yyyymmdd-HHMMSS>
  · 写入后重新读取校验：未触碰的字段与原文语义一致、轨道数 +1、事件数相符、按 start 升序

用法：
    python scripts/gsa_import_embed.py [--project pierro_questions.gsa] [--product pierro_embed_ocr.json] [--dry-run]
"""
import argparse
import io
import json
import os
import random
import shutil
import sys
import time

REPO = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
EXAMPLES = os.path.join(REPO, "examples", "benchmark_examples")
BENCH_OUT = os.path.join(REPO, "temp", "bench_output")
MAGIC = "GSA-PROJECT v1"

ALPHABET = "0123456789abcdefghijklmnopqrstuvwxyz"


def b36(n):
    if n == 0:
        return "0"
    s = ""
    while n:
        n, r = divmod(n, 36)
        s = ALPHABET[r] + s
    return s


def generate_id():
    """复刻 src/stores/project.ts::generateId"""
    return b36(int(time.time() * 1000)) + "".join(random.choice(ALPHABET) for _ in range(6))


def read_gsa(path):
    """→ (magic_line, project_dict, raw_text)"""
    with io.open(path, encoding="utf-8", newline="") as f:
        raw = f.read()
    head, _, body = raw.partition("\n")
    if head.strip() != MAGIC:
        raise SystemExit("首行不是预期魔数 {!r}：{!r}".format(MAGIC, head))
    return head, json.loads(body), raw


def write_gsa(path, project):
    """按 Rust 保存格式写回：魔数行 + 2 空格缩进 JSON + LF + 无 BOM + 末尾换行"""
    body = json.dumps(project, ensure_ascii=False, indent=2)
    with io.open(path, "w", encoding="utf-8", newline="") as f:
        f.write(MAGIC + "\n" + body + "\n")


def build_track(segments, existing_id=None):
    """按 ensureEmbedOcrTrack / embedOcrToEvent 构造产物轨"""
    events = []
    for s in sorted(segments, key=lambda x: x["start"]):
        events.append({
            "type": "embed_ocr",
            "id": generate_id(),
            "start": float(s["start"]),
            "end": float(s["end"]),
            "text": s["text"],
            "confidence": float(s.get("confidence", 0.0)),
        })
    return {
        "id": existing_id or generate_id(),
        "name": "内嵌字幕 OCR",
        "type": "embed_ocr",
        "track_role": "game",
        "scope": "output",
        "page": "",
        "video": "clip",
        "preview_visible": True,
        "events": events,
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--project", default="pierro_questions.gsa")
    ap.add_argument("--product", default="pierro_embed_ocr.json")
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    gsa_path = os.path.join(EXAMPLES, args.project)
    prod_path = os.path.join(BENCH_OUT, args.product)
    if not os.path.exists(gsa_path):
        raise SystemExit("工程不存在：{}".format(gsa_path))
    if not os.path.exists(prod_path):
        raise SystemExit("OCR 产物不存在：{}".format(prod_path))

    with io.open(prod_path, encoding="utf-8") as f:
        product = json.load(f)
    segments = product["segments"]
    if not segments:
        raise SystemExit("OCR 产物 segments 为空，拒绝写入")

    _, project, _ = read_gsa(gsa_path)
    original = json.loads(json.dumps(project))  # 语义快照，用于写入后比对

    print("工程   : {}".format(gsa_path))
    print("产物   : {}（{} 段）".format(prod_path, len(segments)))
    print("现有轨道:")
    for t in project["tracks"]:
        print("   type={:11} role={:8} scope={:8} video={:7} n={:3} name={}".format(
            t.get("type"), str(t.get("track_role")), str(t.get("scope")),
            str(t.get("video")), len(t.get("events", [])), t.get("name")))

    # 幂等：已有 embed_ocr(clip) 轨则整体替换事件（与 writeEmbedOcrSegments 同口径）
    existing = None
    for t in project["tracks"]:
        if t.get("type") == "embed_ocr" and t.get("video") == "clip":
            existing = t
            break

    new_track = build_track(segments, existing.get("id") if existing else None)
    if existing is not None:
        idx = project["tracks"].index(existing)
        project["tracks"][idx] = new_track
        print("\n已有 embed_ocr 轨 → 整体替换事件（id 保留 {}）".format(new_track["id"]))
    else:
        project["tracks"].append(new_track)
        print("\n无 embed_ocr 轨 → 追加到末尾（与 tracks.push 一致）")

    # 保留原始 created_at；updated_at 不主动改（GUI 保存时会自行更新）
    if args.dry_run:
        print("\n[dry-run] 不写盘。将写入 {} 段事件".format(len(new_track["events"])))
        return 0

    stamp = time.strftime("%Y%m%d-%H%M%S")
    backup = "{}.bak-{}".format(gsa_path, stamp)
    shutil.copy2(gsa_path, backup)
    print("\n已备份原文件 → {}".format(os.path.basename(backup)))

    write_gsa(gsa_path, project)

    # ── 写入后校验 ──
    print("\n=== 校验 ===")
    _, check, _ = read_gsa(gsa_path)
    ok = True

    # 1) 未触碰字段语义一致
    for k in original:
        if k == "tracks":
            continue
        if check.get(k) != original.get(k):
            print("  ✗ 字段 {} 发生变化".format(k))
            ok = False
    print("  未触碰字段语义一致: {}".format("✓" if ok else "✗"))

    # 2) 除新轨外的既有轨道逐字未变
    old_others = [t for t in original["tracks"]
                  if not (t.get("type") == "embed_ocr" and t.get("video") == "clip")]
    new_others = [t for t in check["tracks"]
                  if not (t.get("type") == "embed_ocr" and t.get("video") == "clip")]
    same = old_others == new_others
    ok = ok and same
    print("  既有轨道逐字未变: {}（{} 条）".format("✓" if same else "✗", len(new_others)))

    # 3) 产物轨
    got = [t for t in check["tracks"] if t.get("type") == "embed_ocr" and t.get("video") == "clip"]
    if len(got) != 1:
        print("  ✗ embed_ocr 轨数量异常: {}".format(len(got)))
        ok = False
    else:
        tr = got[0]
        ev = tr["events"]
        n_ok = len(ev) == len(segments)
        sorted_ok = all(ev[i]["start"] <= ev[i + 1]["start"] for i in range(len(ev) - 1))
        fields_ok = all(
            set(e.keys()) == {"type", "id", "start", "end", "text", "confidence"}
            and e["type"] == "embed_ocr"
            and isinstance(e["start"], float) and isinstance(e["end"], float)
            and isinstance(e["confidence"], float) and isinstance(e["text"], str)
            and e["text"].strip() != ""
            for e in ev
        )
        ids_ok = len({e["id"] for e in ev}) == len(ev)
        meta_ok = (tr["name"] == "内嵌字幕 OCR" and tr["track_role"] == "game"
                   and tr["scope"] == "output" and tr["page"] == "" and tr["video"] == "clip"
                   and tr["preview_visible"] is True)
        print("  事件数 {} == {}: {}".format(len(ev), len(segments), "✓" if n_ok else "✗"))
        print("  按 start 升序: {}".format("✓" if sorted_ok else "✗"))
        print("  事件字段/类型/非空: {}".format("✓" if fields_ok else "✗"))
        print("  id 唯一: {}".format("✓" if ids_ok else "✗"))
        print("  轨道元数据: {}".format("✓" if meta_ok else "✗"))
        ok = ok and n_ok and sorted_ok and fields_ok and ids_ok and meta_ok
        print("  时间范围: {:.2f}s → {:.2f}s".format(ev[0]["start"], ev[-1]["end"]))

    print("\n结果: {}".format("✓ 全部通过" if ok else "✗ 存在失败项，请检查（原文件已备份）"))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
