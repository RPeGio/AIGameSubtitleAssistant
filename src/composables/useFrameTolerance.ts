import type { VideoMetadata } from "../types";

/// 一帧的时长（秒）。没有视频元数据时按 30fps 保守估计。
export function frameDurOf(meta: VideoMetadata | null | undefined): number {
  const fps = meta?.fps ?? 0;
  return 1 / (fps > 0 ? fps : 30);
}

/// 事件是否落在「播放头当前显示的那一帧」内。
///
/// 不能用硬边界（start <= t）：<video> 的 seek 落点是帧 PTS，可能比请求的跳转
/// 目标早最多一帧，于是「跳到 clip 起点」后 store 的时间会落在 start 之前一点点，
/// 该 clip 的字幕（SubtitleOverlay）、选区（RegionOverlay）与读数会整段消失。
/// 语义：播放头所在帧覆盖 [t, t + frameDur)，起始落在这帧里的事件就该显示。
export function hitsFrame(start: number, end: number, t: number, frameDur: number): boolean {
  return start < t + frameDur && end > t;
}
