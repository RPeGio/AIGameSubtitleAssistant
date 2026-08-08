import { defineStore } from "pinia";
import { onScopeDispose, ref, watch } from "vue";
import type {
  Project,
  RecentProject,
  VideoMetadata,
  TimelineEvent,
  Track,
  OcrRunParams,
  OcrSegment,
  OcrProgress,
  AsrSegment,
  AsrProgress,
} from "../types";
import { OCR_PROGRESS_EVENT, ASR_PROGRESS_EVENT } from "../types";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";

function generateId(): string {
  return Date.now().toString(36) + Math.random().toString(36).slice(2, 8);
}

/// 无说话人标签时的轨道名/事件字段 fallback
const UNKNOWN_SPEAKER = "未标注";

function ocrTextToEvent(seg: OcrSegment): TimelineEvent {
  return {
    id: generateId(),
    type: "ocr_text",
    start: seg.start,
    end: seg.end,
    text: seg.text,
    confidence: seg.confidence,
  };
}

function asrSegmentToEvent(seg: AsrSegment): TimelineEvent {
  return {
    id: generateId(),
    type: "asr",
    start: seg.start,
    end: seg.end,
    text: seg.text,
    speaker: seg.speaker || UNKNOWN_SPEAKER,
    confidence: seg.confidence ?? 0,
  };
}

export const useProjectStore = defineStore("project", () => {
  const currentProject = ref<Project | null>(null);
  const recentProjects = ref<RecentProject[]>([]);
  const currentVideoMeta = ref<VideoMetadata | null>(null);
  const isLoading = ref(false);
  const videoImportError = ref<string | null>(null);

  // ── OCR 运行状态 ───────────────────────────────────────
  const ocrRunning = ref(false);
  const ocrProgress = ref(0);
  const ocrMessage = ref("");

  // 监听进度事件；注册清理函数，store 被 dispose（HMR/重复实例）时退订，避免叠加泄漏
  const unlistenPromise = listen<OcrProgress>(OCR_PROGRESS_EVENT, (event) => {
    ocrProgress.value = event.payload.progress;
    ocrMessage.value = event.payload.message;
  });
  unlistenPromise.catch((e) => console.error("监听 OCR 进度事件失败:", e));
  onScopeDispose(() => {
    unlistenPromise.then((fn) => fn()).catch(() => {});
  });

  // ── ASR 运行状态 ───────────────────────────────────────
  const asrRunning = ref(false);
  const asrProgress = ref(0);
  const asrMessage = ref("");

  // 监听进度事件；注册清理函数，store 被 dispose（HMR/重复实例）时退订，避免叠加泄漏
  const asrUnlistenPromise = listen<AsrProgress>(ASR_PROGRESS_EVENT, (event) => {
    asrProgress.value = event.payload.progress;
    asrMessage.value = event.payload.message;
  });
  asrUnlistenPromise.catch((e) => console.error("监听 ASR 进度事件失败:", e));
  onScopeDispose(() => {
    asrUnlistenPromise.then((fn) => fn()).catch(() => {});
  });

  // ── 自动保存 ─────────────────────────────────────────
  const saveState = ref<"saved" | "pending" | "saving" | "error">("saved");
  let saveTimer: number | undefined;
  // 保存后把 updated_at 同步回 currentProject，会触发深监听；
  // 用该标记短路，避免"保存→触发监听→再保存"的死循环
  let applyingSaved = false;

  /// 保存结果：ok=成功 | failed=写入失败 | skipped=无可保存（无项目/加载中/保存期间项目已切换）
  async function saveNow(): Promise<"ok" | "failed" | "skipped"> {
    const project = currentProject.value;
    if (isLoading.value || !project) return "skipped";
    saveState.value = "saving";
    try {
      const updated = await invoke<Project>("save_project", {
        project,
      });
      // await 期间项目可能被关闭/切换（async 竞态），此时丢弃本次合并结果
      if (isLoading.value || !currentProject.value || currentProject.value !== project) {
        return "skipped";
      }
      applyingSaved = true;
      // 只合并 updated_at，不整体替换 —— 避免覆盖保存在响应式对象里的并发改动
      currentProject.value.updated_at = updated.updated_at;
      applyingSaved = false;
      saveState.value = "saved";
      return "ok";
    } catch (e) {
      saveState.value = "error";
      console.error("项目保存失败:", e);
      return "failed";
    }
  }

  function scheduleSave() {
    saveState.value = "pending";
    clearTimeout(saveTimer);
    saveTimer = window.setTimeout(saveNow, 1000);
  }

  // 深监听整个项目对象：任何轨道/clip/坐标变更都会触发自动保存
  watch(
    () => currentProject.value,
    () => {
      if (isLoading.value || applyingSaved) return;
      scheduleSave();
    },
    { deep: true, flush: "sync" }
  );

  // 应用关闭前尽力 flush（Tauri 关闭窗口时 WebView 可能直接销毁，best-effort）
  window.addEventListener("beforeunload", () => {
    clearTimeout(saveTimer);
    saveNow();
  });

  async function createProject(name: string, path: string) {
    isLoading.value = true;
    try {
      const project = await invoke<Project>("create_project", { name, path });
      currentProject.value = project;
      await refreshRecentProjects();
      return project;
    } finally {
      isLoading.value = false;
    }
  }

  async function openProject(path: string) {
    isLoading.value = true;
    try {
      const project = await invoke<Project>("open_project", { path });
      currentProject.value = project;
      if (project.video) {
        await loadVideoMeta(project.video);
      }
      await refreshRecentProjects();
      return project;
    } finally {
      isLoading.value = false;
    }
  }

  async function importVideo() {
    videoImportError.value = null;

    const selected = await open({
      multiple: false,
      title: "选择视频文件",
      filters: [
        {
          name: "视频文件",
          extensions: ["mp4", "mkv", "webm", "avi", "mov", "flv"],
        },
      ],
    });

    if (!selected) return;

    await loadVideoMeta(selected);
  }

  async function loadVideoMeta(videoPath: string) {
    try {
      const meta = await invoke<VideoMetadata>("get_video_metadata", {
        path: videoPath,
      });
      currentVideoMeta.value = meta;

      if (currentProject.value) {
        const updated = await invoke<Project>("set_project_video", {
          projectPath: currentProject.value.path,
          videoPath,
        });
        // 只合并字段，不整体替换 —— 防止并发写入的轨道（ensureDefaultTrack 等在
        // loadedmetadata 后添加）被 set_project_video 返回的旧对象覆盖丢失
        currentProject.value.video = updated.video;
        currentProject.value.updated_at = updated.updated_at;
      }
    } catch (e) {
      const err = String(e);
      if (err.includes("FFMPEG_NOT_FOUND")) {
        videoImportError.value = "FFmpeg 未安装，请在项目设置中下载运行环境";
      } else {
        videoImportError.value = err;
      }
    }
  }

  async function refreshRecentProjects() {
    try {
      recentProjects.value = await invoke<RecentProject[]>("list_recent_projects");
    } catch (e) {
      recentProjects.value = [];
    }
  }

  function ensureDefaultTrack(duration: number) {
    if (!currentProject.value) return;
    const tracks = currentProject.value.tracks;

    // OCR 选区轨道：默认一个覆盖整段视频的选区
    if (!tracks.some((t) => t.type === "ocr_region")) {
      tracks.push({
        id: generateId(),
        name: "OCR 选区",
        type: "ocr_region",
        events: [
          {
            id: generateId(),
            start: 0,
            end: duration,
            type: "ocr_region",
            // 默认矩形：宽 60%，高 20%，水平居中，保持在画面偏低位置
            x1: 0.2,
            y1: 0.7,
            x2: 0.8,
            y2: 0.9,
          },
        ],
      });
    }

    if (!tracks.some((t) => t.type === "ocr_text")) {
      tracks.push({
        id: generateId(),
        name: "剧情文本 (mock)",
        type: "ocr_text",
        events: [
          {
            id: generateId(),
            type: "ocr_text",
            start: 0.5,
            end: duration * 0.1,
            text: "旅行者，你来了",
            confidence: 0.88,
          },
          {
            id: generateId(),
            type: "ocr_text",
            start: duration * 0.45,
            end: duration * 0.55,
            text: "前方似乎有什么东西在等待",
            confidence: 0.9,
          },
        ],
      });
    }
  }

  /// 根据事件 id 跨所有轨道查找 { track, event }
  function findEvent(id: string | null): { track: Track; event: TimelineEvent } | null {
    if (!currentProject.value || !id) return null;
    for (const track of currentProject.value.tracks) {
      const event = track.events.find((e) => e.id === id);
      if (event) return { track, event };
    }
    return null;
  }

  /// 根据轨道 id 查找轨道
  function findTrack(id: string | null): Track | null {
    if (!currentProject.value || !id) return null;
    return currentProject.value.tracks.find((t) => t.id === id) ?? null;
  }

  /// 在 time 处把事件切成两段，两段继承全部字段（含矩形坐标），返回右段 id
  function splitEvent(eventId: string, time: number): string | null {
    const found = findEvent(eventId);
    if (!found) return null;
    const { track, event } = found;
    // 保护：分割点必须在区间内部，避免产生零长度 sliver
    const EPS = 0.05;
    if (time <= event.start + EPS || time >= event.end - EPS) return null;

    const left = { ...event, id: generateId(), end: time };
    const right = { ...event, id: generateId(), start: time };
    const idx = track.events.indexOf(event);
    track.events.splice(idx, 1, left, right);
    track.events.sort((a, b) => a.start - b.start);
    return right.id;
  }

  /// 更新指定 ocr_region 事件的坐标（就地修改 reactive 对象，时间轴即时刷新）
  function updateOcrRegion(
    id: string,
    coords: { x1: number; y1: number; x2: number; y2: number }
  ) {
    const found = findEvent(id);
    if (found && found.event.type === "ocr_region") {
      found.event.x1 = coords.x1;
      found.event.y1 = coords.y1;
      found.event.x2 = coords.x2;
      found.event.y2 = coords.y2;
    }
  }

  /// 更新带 text 字段的事件的文字（ocr_text / asr / manual，就地修改 reactive 对象）
  function updateEventText(id: string, text: string) {
    const found = findEvent(id);
    if (!found) return;
    if (found.event.type !== "ocr_region") {
      found.event.text = text;
    }
  }

  /// 运行 OCR 流水线：收集所有 ocr_region 轨道的 clip → run_ocr → 写入 ocr_text 轨道
  async function runOcr(params: OcrRunParams) {
    if (ocrRunning.value || !currentProject.value || !currentVideoMeta.value) return;

    const regionClips = currentProject.value.tracks
      .filter((t) => t.type === "ocr_region")
      .flatMap((t) => t.events)
      .filter((e): e is Extract<typeof e, { type: "ocr_region" }> => e.type === "ocr_region")
      .map((e) => ({
        start: e.start,
        end: e.end,
        x1: e.x1,
        y1: e.y1,
        x2: e.x2,
        y2: e.y2,
      }))
      .sort((a, b) => a.start - b.start);

    if (regionClips.length === 0) {
      throw new Error("请先设置 OCR 选区");
    }

    ocrRunning.value = true;
    ocrProgress.value = 0;
    ocrMessage.value = "准备中...";
    try {
      const segments = await invoke<OcrSegment[]>("run_ocr", {
        videoPath: currentVideoMeta.value.path,
        videoW: currentVideoMeta.value.width,
        videoH: currentVideoMeta.value.height,
        regionClips,
        params,
      });
      writeOcrSegments(segments);
      ocrProgress.value = 1;
      ocrMessage.value = "完成";
    } catch (e) {
      // 失败时重置进度并抛出友好错误，避免 UI 残留半程状态
      ocrProgress.value = 0;
      ocrMessage.value = "OCR 失败";
      throw new Error(normalizeOcrError(e));
    } finally {
      ocrRunning.value = false;
    }
  }

  /// 把 IPC/后端错误映射为用户可读的信息，未识别时才回退原文
  function normalizeOcrError(e: unknown): string {
    const msg = String(e);
    if (msg.includes("未就绪")) return "OCR 运行环境未就绪，请先运行环境引导脚本";
    if (msg.includes("请先设置 OCR 选区")) return "请先设置 OCR 选区";
    return msg;
  }

  /// 复用或新建 ocr_text 轨道
  function ensureOcrTextTrack(): Track {
    let track = currentProject.value?.tracks.find((t) => t.type === "ocr_text");
    if (!track && currentProject.value) {
      track = {
        id: generateId(),
        name: "OCR 文本",
        type: "ocr_text",
        events: [],
      };
      currentProject.value.tracks.push(track);
    }
    if (!track) throw new Error("当前无项目");
    return track;
  }

  /// 清空并填充 ocr_text 轨道的事件（重跑不叠加）
  function writeOcrSegments(segments: OcrSegment[]) {
    const track = ensureOcrTextTrack();
    track.events = segments.map(ocrTextToEvent).sort((a, b) => a.start - b.start);
  }

  /// 运行 ASR 流水线：整段视频 → run_asr → 按 speaker 分组写入各 asr 轨道
  async function runAsr() {
    if (asrRunning.value || !currentProject.value || !currentVideoMeta.value) return;

    asrRunning.value = true;
    asrProgress.value = 0;
    asrMessage.value = "准备中...";
    try {
      const segments = await invoke<AsrSegment[]>("run_asr", {
        videoPath: currentVideoMeta.value.path,
      });
      writeAsrSegments(segments);
      asrProgress.value = 1;
      asrMessage.value = "完成";
    } catch (e) {
      // 失败/中断时不动已有轨道，只重置进度并抛出友好错误
      asrProgress.value = 0;
      asrMessage.value = "ASR 失败";
      throw new Error(normalizeAsrError(e));
    } finally {
      asrRunning.value = false;
    }
  }

  /// 把 IPC/后端错误映射为用户可读的信息，未识别时才回退原文
  function normalizeAsrError(e: unknown): string {
    const msg = String(e);
    if (msg.includes("未就绪")) return "ASR 运行环境未就绪，请先运行环境引导脚本";
    return msg;
  }

  /// 说话人一条 asr 轨道：找到同 speaker 的轨道复用，否则新建（轨道名 = speaker 标签）
  function ensureAsrTrack(speaker: string): Track {
    if (!currentProject.value) throw new Error("当前无项目");
    const tracks = currentProject.value.tracks;
    let track = tracks.find((t) => t.type === "asr" && t.name === speaker);
    if (!track) {
      track = {
        id: generateId(),
        name: speaker,
        type: "asr",
        events: [],
      };
      tracks.push(track);
    }
    return track;
  }

  /// ASR 成功后按 speaker 分组填充各 asr 轨道，重跑不叠加。
  /// 空结果视为无可识别语音，不动已有轨道（保护历史数据）。
  function writeAsrSegments(segments: AsrSegment[]) {
    if (!currentProject.value || segments.length === 0) return;
    currentProject.value.tracks
      .filter((t) => t.type === "asr")
      .forEach((t) => (t.events = []));

    const bySpeaker = new Map<string, TimelineEvent[]>();
    for (const seg of segments) {
      const speaker = seg.speaker || UNKNOWN_SPEAKER;
      if (!bySpeaker.has(speaker)) bySpeaker.set(speaker, []);
      bySpeaker.get(speaker)!.push(asrSegmentToEvent(seg));
    }
    for (const [speaker, events] of bySpeaker) {
      const track = ensureAsrTrack(speaker);
      track.events = events.sort((a, b) => a.start - b.start);
    }
    // 清理重跑后不再出现/无事件的 asr 轨道，避免空壳残留
    currentProject.value.tracks = currentProject.value.tracks.filter(
      (t) => t.type !== "asr" || t.events.length > 0
    );
  }

  /// 重命名轨道：asr/manual 事件的 character 跟随轨道角色名（空名不改名）
  function renameTrack(trackId: string, name: string) {
    const track = currentProject.value?.tracks.find((t) => t.id === trackId);
    if (!track) return;
    const trimmed = name.trim();
    if (trimmed.length === 0) return;
    track.name = trimmed;
    for (const e of track.events) {
      if (e.type === "asr" || e.type === "manual") {
        e.character = trimmed;
      }
    }
  }

  function closeProject() {
    // 关闭前落盘（在置空前触发保存）
    clearTimeout(saveTimer);
    saveNow();
    currentProject.value = null;
    currentVideoMeta.value = null;
    videoImportError.value = null;
    saveState.value = "saved";
  }

  return {
    currentProject,
    recentProjects,
    currentVideoMeta,
    isLoading,
    videoImportError,
    saveState,
    createProject,
    openProject,
    importVideo,
    ensureDefaultTrack,
    findEvent,
    findTrack,
    renameTrack,
    splitEvent,
    updateOcrRegion,
    updateEventText,
    refreshRecentProjects,
    saveNow,
    ocrRunning,
    ocrProgress,
    ocrMessage,
    runOcr,
    asrRunning,
    asrProgress,
    asrMessage,
    runAsr,
    closeProject,
  };
});
