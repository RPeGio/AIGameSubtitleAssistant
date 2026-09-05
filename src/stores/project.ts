import { defineStore } from "pinia";
import { computed, onScopeDispose, ref, watch } from "vue";
import type {
  Project,
  RecentProject,
  VideoMetadata,
  TimelineEvent,
  Track,
  CorpusItem,
  OcrRunParams,
  OcrSegment,
  OcrProgress,
  AsrRunParams,
  AsrSegment,
  AsrEngineStatus,
  AsrProgress,
  LlmProgress,
  LlmRuntimeStatus,
  FuseAsrInput,
  FuseResult,
  FusedSegment,
} from "../types";
import { OCR_PROGRESS_EVENT, ASR_PROGRESS_EVENT, LLM_PROGRESS_EVENT } from "../types";
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

function fusedToEvent(seg: FusedSegment): TimelineEvent {
  return {
    id: generateId(),
    type: "fused",
    start: seg.start,
    end: seg.end,
    text: seg.text,
    ...(seg.character ? { character: seg.character } : {}),
  };
}

export const useProjectStore = defineStore("project", () => {
  const currentProject = ref<Project | null>(null);
  const recentProjects = ref<RecentProject[]>([]);
  /// 切片视频（时间轴基准）元数据 —— 全局时间轴跟随它
  const currentVideoMeta = ref<VideoMetadata | null>(null);
  /// 剧情录屏（文本源）元数据 —— 语料页 OCR 用，独立于全局时间轴
  const sourceVideoMeta = ref<VideoMetadata | null>(null);
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

  // ── LLM 运行状态 ───────────────────────────────────────
  const llmRunning = ref(false);
  const llmProgress = ref(0);
  const llmMessage = ref("");

  // 监听进度事件；注册清理函数，store 被 dispose（HMR/重复实例）时退订，避免叠加泄漏
  const llmUnlistenPromise = listen<LlmProgress>(LLM_PROGRESS_EVENT, (event) => {
    llmProgress.value = event.payload.progress;
    llmMessage.value = event.payload.message;
  });
  llmUnlistenPromise.catch((e) => console.error("监听 LLM 进度事件失败:", e));
  onScopeDispose(() => {
    llmUnlistenPromise.then((fn) => fn()).catch(() => {});
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

  // ── 撤销/重做：快照式操作记录 ─────────────────────────
  // 每次操作前把当前 tracks 深拷贝入 undo 栈（一个操作 = 一个撤销步骤）；
  // 高频写入（拖动改时/区域拖动/文字编辑）在操作开始点记录，写入中不重复记录
  /// 快照结构：tracks 与 corpus 一起入栈——语料的增删也走撤销栈
  type UndoSnapshot = { tracks: Track[]; corpus: CorpusItem[] };
  const undoStack = ref<UndoSnapshot[]>([]);
  const redoStack = ref<UndoSnapshot[]>([]);
  const MAX_HISTORY = 60;

  function snapshot(): UndoSnapshot {
    const p = currentProject.value;
    return {
      tracks: JSON.parse(JSON.stringify(p?.tracks ?? [])) as Track[],
      corpus: JSON.parse(JSON.stringify(p?.corpus ?? [])) as CorpusItem[],
    };
  }

  /// 操作执行前调用：当前状态入 undo 栈，新操作打断重做链
  function recordSnapshot() {
    if (!currentProject.value) return;
    undoStack.value.push(snapshot());
    if (undoStack.value.length > MAX_HISTORY) undoStack.value.shift();
    redoStack.value = [];
  }

  function undo() {
    if (!currentProject.value || undoStack.value.length === 0) return;
    redoStack.value.push(snapshot());
    const snap = undoStack.value.pop()!;
    currentProject.value.tracks = snap.tracks;
    currentProject.value.corpus = snap.corpus;
  }

  function redo() {
    if (!currentProject.value || redoStack.value.length === 0) return;
    undoStack.value.push(snapshot());
    const snap = redoStack.value.pop()!;
    currentProject.value.tracks = snap.tracks;
    currentProject.value.corpus = snap.corpus;
  }

  function clearHistory() {
    undoStack.value = [];
    redoStack.value = [];
  }

  const canUndo = computed(() => undoStack.value.length > 0);
  const canRedo = computed(() => redoStack.value.length > 0);

  async function createProject(name: string, path: string) {
    isLoading.value = true;
    try {
      const project = await invoke<Project>("create_project", { name, path });
      currentProject.value = project;
      clearHistory();
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
      clearHistory();
      if (project.video) {
        await loadVideoMeta(project.video);
      }
      if (project.source_video) {
        try {
          const smeta = await invoke<VideoMetadata>("get_video_metadata", {
            path: project.source_video,
          });
          sourceVideoMeta.value = smeta;
          if (smeta.duration > 0) ensureCorpusRegionTrack(smeta.duration);
        } catch {
          sourceVideoMeta.value = null;
        }
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

  /// 导入剧情录屏（文本源视频 A），仅写 source_video 字段，不触碰全局时间轴。
  /// 导入成功后统一确保 source 选区控制轨存在（单一建轨入口，避免多处重复创建）
  async function importSourceVideo() {
    const selected = await open({
      multiple: false,
      title: "选择剧情录屏视频",
      filters: [
        {
          name: "视频文件",
          extensions: ["mp4", "mkv", "webm", "avi", "mov", "flv"],
        },
      ],
    });
    if (!selected || !currentProject.value) return;

    try {
      const meta = await invoke<VideoMetadata>("get_video_metadata", {
        path: selected,
      });
      sourceVideoMeta.value = meta;
      currentProject.value.source_video = selected;
      currentProject.value.updated_at = new Date().toISOString();
      // 导入成功即建轨（幂等守卫），语料页 watch 与预览组件不再各自创建
      if (meta.duration > 0) ensureCorpusRegionTrack(meta.duration);
    } catch (e) {
      const err = String(e);
      videoImportError.value = err.includes("FFMPEG_NOT_FOUND")
        ? "FFmpeg 未安装，请在项目设置中下载运行环境"
        : err;
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

    // OCR 选区轨道：控制轨（页面工作状态），绑定切片视频，归 editor 页。
    // 精确匹配 video!=="source"：避免语料页的 source 控制轨（同为 ocr_region）被误判为已存在
    if (!tracks.some((t) => t.type === "ocr_region" && t.video !== "source")) {
      tracks.push({
        id: generateId(),
        name: "OCR 选区",
        type: "ocr_region",
        track_role: "game",
        scope: "control",
        page: "editor",
        video: "clip",
        preview_visible: true,
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
        track_role: "game",
        scope: "output",
        page: "",
        video: "clip",
        preview_visible: true,
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

  /// 语料页：确保剧情录屏（视频 A）的 OCR 选区控制轨存在。
  /// 默认一个覆盖整段的选区（任意播放头都能命中，可直接拖拽调整）。
  /// 守卫按 (type=ocr_region) + (video=source 或 page=corpus 或轨道名) 匹配：
  /// 兼容 video 字段缺失的旧数据，避免更换视频时重复创建。
  function ensureCorpusRegionTrack(duration: number) {
    if (!currentProject.value) return;
    const tracks = currentProject.value.tracks;
    const exists = tracks.some(
      (t) =>
        t.type === "ocr_region" &&
        (t.video === "source" || t.page === "corpus" || t.name === "OCR 选区（剧情录屏）")
    );
    if (exists) return;
    tracks.push({
      id: generateId(),
      name: "OCR 选区（剧情录屏）",
      type: "ocr_region",
      track_role: "game",
      scope: "control",
      page: "corpus",
      video: "source",
      preview_visible: true,
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

    recordSnapshot();
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

  /// 在指定 ocr_region 轨上新增一条选区事件（时间轴双击空白处添加不同时间段的选区）。
  /// 默认矩形与 ensureCorpusRegionTrack 一致（宽 60%、高 20%、画面偏低居中）。
  function addOcrRegionEvent(
    trackId: string,
    start: number,
    end: number,
    coords?: { x1: number; y1: number; x2: number; y2: number }
  ) {
    if (!currentProject.value || end <= start) return;
    const track = currentProject.value.tracks.find((t) => t.id === trackId);
    if (!track) return;
    recordSnapshot();
    track.events.push({
      id: generateId(),
      start,
      end,
      type: "ocr_region",
      x1: coords?.x1 ?? 0.2,
      y1: coords?.y1 ?? 0.7,
      x2: coords?.x2 ?? 0.8,
      y2: coords?.y2 ?? 0.9,
    });
    track.events.sort((a, b) => a.start - b.start);
  }

  /// 更新带 text 字段的事件的文字（ocr_text / asr / manual，就地修改 reactive 对象）
  function updateEventText(id: string, text: string) {
    const found = findEvent(id);
    if (!found) return;
    if (found.event.type !== "ocr_region") {
      found.event.text = text;
    }
  }

  /// 更新事件起止时间（就地修改，时间轴即时刷新），并按 start 重排序
  function updateEventTime(id: string, start: number, end: number) {
    const found = findEvent(id);
    if (!found || end <= start) return;
    found.event.start = start;
    found.event.end = end;
    found.track.events = [...found.track.events].sort((a, b) => a.start - b.start);
  }

  /// 运行 OCR 流水线：收集所有 ocr_region 轨道的 clip → run_ocr → 写入 ocr_text 轨道
  /// `videoKey`：默认 "clip"（切片，编辑页 OCR）；"source" 用剧情录屏（语料页 OCR，meta 取 sourceVideoMeta）
  async function runOcr(params: OcrRunParams, videoKey: "clip" | "source" = "clip") {
    if (ocrRunning.value) return;
    // meta 缺失也要给调用方可展示的错误，不能静默返回
    if (!currentProject.value) throw new Error("请先打开项目");
    const meta = videoKey === "source" ? sourceVideoMeta.value : currentVideoMeta.value;
    if (!meta) {
      throw new Error(videoKey === "source" ? "请先导入剧情录屏" : "请先导入切片视频");
    }

    // 语料页只取挂在剧情录屏上的 OCR 选区控制轨；编辑页沿用全部选区
    const regionClips = currentProject.value.tracks
      .filter(
        (t) =>
          t.type === "ocr_region" &&
          (videoKey === "source" ? t.video === "source" : t.video !== "source")
      )
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
        videoPath: meta.path,
        videoW: meta.width,
        videoH: meta.height,
        srcFps: meta.fps,
        regionClips,
        params,
      });
      if (videoKey === "source") {
        // 语料页：产物提取进 corpus（去时间轴，作为可靠文本语料）
        writeOcrToCorpus(segments);
      } else {
        writeOcrSegments(segments);
      }
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
        track_role: "game",
        scope: "output",
        page: "",
        video: "clip",
        preview_visible: true,
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
    recordSnapshot();
    track.events = segments.map(ocrTextToEvent).sort((a, b) => a.start - b.start);
  }

  // ── 文本语料（corpus）─────────────────────────────────

  /// 把 OCR 段文本提取进 corpus（去时间轴，去重），供融合页消费
  function writeOcrToCorpus(segments: OcrSegment[]) {
    if (!currentProject.value) return;
    const project = currentProject.value;
    const existing = new Set(project.corpus.map((c) => c.text));
    const now = new Date().toISOString();
    const added: CorpusItem[] = [];
    for (const seg of segments) {
      const text = seg.text.trim();
      if (!text || existing.has(text)) continue;
      existing.add(text);
      added.push({ id: generateId(), text, source: "ocr_track", created_at: now });
    }
    if (added.length > 0) {
      recordSnapshot();
      project.corpus.push(...added);
    }
  }

  /// 手动添加一条语料（粘贴文本）
  function addCorpusItem(text: string, source: CorpusItem["source"] = "paste") {
    if (!currentProject.value || !text.trim()) return;
    recordSnapshot();
    currentProject.value.corpus.push({
      id: generateId(),
      text: text.trim(),
      source,
      created_at: new Date().toISOString(),
    });
  }

  /// 删除一条语料
  function removeCorpusItem(id: string) {
    if (!currentProject.value) return;
    recordSnapshot();
    currentProject.value.corpus = currentProject.value.corpus.filter((c) => c.id !== id);
  }

  /// 语料就绪：corpus 非空
  const corpusReady = computed(
    () => (currentProject.value?.corpus.length ?? 0) > 0
  );

  /// 时间轴就绪：存在已标记为游戏内容（track_role=game）且非空的 ASR 轨。
  /// 融合只消费游戏内语音（嵌字轴），主播语音轨（streamer）不参与，
  /// 因此仅有主播轨时视为未就绪。
  const timelineReady = computed(() =>
    (currentProject.value?.tracks ?? []).some(
      (t) => t.type === "asr" && t.track_role === "game" && t.events.length > 0
    )
  );

  /// 运行 ASR 流水线：整段视频 → run_asr（面板参数：引擎/说话人上限/语言）→ 按 speaker 分组写入各 asr 轨道
  async function runAsr(params: AsrRunParams) {
    if (asrRunning.value || !currentProject.value || !currentVideoMeta.value) return;

    asrRunning.value = true;
    asrProgress.value = 0;
    asrMessage.value = "准备中...";
    try {
      const segments = await invoke<AsrSegment[]>("run_asr", {
        videoPath: currentVideoMeta.value.path,
        params,
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

  /// 探测两个 ASR 引擎（funasr/moss）的运行环境（供配置面板禁用不可用引擎）
  async function getAsrEngines(): Promise<AsrEngineStatus[]> {
    return invoke<AsrEngineStatus[]>("check_asr_engines");
  }

  /// 请求取消当前 ASR 转写（后端终止 MOSS 子进程，runAsr 的 invoke 会返回"已取消"错误）
  async function cancelAsr() {
    try {
      await invoke("asr_cancel");
    } catch (e) {
      console.error("取消 ASR 失败:", e);
    }
  }

  /// 把 IPC/后端错误映射为用户可读的信息，未识别时才回退原文
  function normalizeAsrError(e: unknown): string {
    const msg = String(e);
    if (msg.includes("已取消")) return "ASR 已取消";
    // 超时消息兼容 MOSS / FunASR 两个 provider
    if (msg.includes("超时")) return msg.replace(/(?:MOSS|FunASR) 转写超时（\d+ 分钟），已终止子进程/, "ASR 转写超时，已终止");
    if (msg.includes("未就绪")) return "ASR 运行环境未就绪，请先运行环境引导脚本";
    return msg;
  }

  /// 执行一次 LLM 推理（手动 prompt 测试台），返回回答文本
  async function runLlm(prompt: string): Promise<string> {
    if (llmRunning.value) throw new Error("LLM 正在运行中");
    if (!prompt.trim()) throw new Error("请输入 prompt");

    llmRunning.value = true;
    llmProgress.value = 0;
    llmMessage.value = "准备中...";
    try {
      const answer = await invoke<string>("run_llm", { prompt });
      llmProgress.value = 1;
      llmMessage.value = "完成";
      return answer;
    } catch (e) {
      // 失败时重置进度并抛出友好错误，避免 UI 残留半程状态
      llmProgress.value = 0;
      llmMessage.value = "LLM 推理失败";
      throw new Error(normalizeLlmError(e));
    } finally {
      llmRunning.value = false;
    }
  }

  /// 探测 LLM 运行环境（面板打开时展示状态）
  async function checkLlmRuntime(): Promise<LlmRuntimeStatus> {
    return await invoke<LlmRuntimeStatus>("check_llm_runtime");
  }

  /// 把 IPC/后端错误映射为用户可读的信息，未识别时才回退原文
  function normalizeLlmError(e: unknown): string {
    const msg = String(e);
    if (msg.includes("未就绪")) return "LLM 运行环境未就绪，请先运行环境引导脚本";
    return msg;
  }

  // ── AI 融合（Phase 4）──────────────────────────────────
  const fuseRunning = ref(false);

  /// 收集 corpus 语料文本（优先）与任意 ASR 段（按时间排序、编号），
  /// 调 run_fuse 交给 LLM 融合，结果写入 fused 最终产物轨道（重跑覆盖）
  async function runFuse(): Promise<FuseResult> {
    if (fuseRunning.value || !currentProject.value) throw new Error("当前无法执行 AI 融合");
    if (!currentVideoMeta.value) throw new Error("请先导入视频");

    // 可靠文本：优先用 corpus 语料；无语料时回退 ocr_text 轨（旧数据流兼容）
    const project = currentProject.value;
    let ocrTexts: string[];
    if (project.corpus.length > 0) {
      ocrTexts = project.corpus.map((c) => c.text);
    } else {
      const ocrTrack = project.tracks.find((t) => t.type === "ocr_text");
      ocrTexts = (ocrTrack?.events ?? [])
        .filter((e): e is Extract<TimelineEvent, { type: "ocr_text" }> => e.type === "ocr_text")
        .map((e) => e.text);
    }

    // 转写文本：只取已标记为游戏内容（track_role=game）的 ASR 轨。
    // 主播语音轨（streamer）不参与融合（嵌字轴只替换游戏内语音），
    // 与 timelineReady 的就绪判定保持一致。
    const asrSegments: FuseAsrInput[] = project.tracks
      .filter((t) => t.type === "asr" && t.track_role === "game")
      .flatMap((t) => t.events)
      .filter((e): e is Extract<TimelineEvent, { type: "asr" }> => e.type === "asr")
      .sort((a, b) => a.start - b.start)
      .map((e, i) => ({
        index: i + 1,
        start: e.start,
        end: e.end,
        text: e.text,
      }));

    fuseRunning.value = true;
    llmProgress.value = 0;
    llmMessage.value = "准备中...";
    try {
      const result = await invoke<FuseResult>("run_fuse", { ocrTexts, asrSegments });
      writeFusedSegments(result);
      llmProgress.value = 1;
      llmMessage.value = "完成";
      return result;
    } catch (e) {
      llmProgress.value = 0;
      llmMessage.value = "AI 融合失败";
      throw new Error(normalizeFuseError(e));
    } finally {
      fuseRunning.value = false;
    }
  }

  /// 复用或新建 fused 最终产物轨道
  function ensureFusedTrack(): Track {
    let track = currentProject.value?.tracks.find((t) => t.type === "fused");
    if (!track && currentProject.value) {
      track = {
        id: generateId(),
        name: "最终字幕",
        type: "fused",
        track_role: "game",
        scope: "output",
        page: "",
        video: "clip",
        preview_visible: true,
        events: [],
      };
      currentProject.value.tracks.push(track);
    }
    if (!track) throw new Error("当前无项目");
    return track;
  }

  /// 清空并填充 fused 轨道的事件（重跑不叠加）。
  /// 快照先于 ensureFusedTrack：首次运行时撤销能把新建的轨道一并移除
  function writeFusedSegments(result: FuseResult) {
    recordSnapshot();
    const track = ensureFusedTrack();
    track.events = result.segments.map(fusedToEvent).sort((a, b) => a.start - b.start);
  }

  /// 把 IPC/后端错误映射为用户可读的信息，未识别时才回退原文
  function normalizeFuseError(e: unknown): string {
    const msg = String(e);
    if (msg.includes("未就绪")) return "LLM 运行环境未就绪，请先运行环境引导脚本";
    if (msg.includes("没有可用的 OCR")) return "缺少 OCR 字幕文本，请先运行 OCR";
    if (msg.includes("没有可用的游戏内容"))
      return "没有游戏内容 ASR 段，请先运行 ASR 并检查轨道属性（主播语音除外）";
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
        // 默认游戏内容：主播语音轨由用户手动标记（run_fuse 只消费游戏内容轨）
        track_role: "game",
        scope: "output",
        page: "",
        video: "clip",
        preview_visible: true,
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
    recordSnapshot();
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

  /// 把事件移动到目标轨道（asr 事件垂直拖拽换轨：人工修正说话人归属）。
  /// 约束：事件与目标轨道均为 asr 类型；character 跟随目标轨道名（空名不改）。
  /// record 为 false 时调用方负责已记录快照（如水平拖动已记录），避免一次拖动两个撤销步骤。
  /// 返回是否成功。speaker 原始标签保留（不污染 ASR 数据）。
  function moveEventToTrack(
    eventId: string,
    targetTrackId: string,
    record: boolean = true
  ): boolean {
    if (!currentProject.value) return false;
    const src = findEvent(eventId);
    if (!src || src.event.type !== "asr") return false;
    const dst = currentProject.value.tracks.find((t) => t.id === targetTrackId);
    if (!dst || dst.type !== "asr" || dst.id === src.track.id) return false;

    if (record) recordSnapshot();
    src.track.events = src.track.events.filter((e) => e.id !== eventId);
    dst.events = [...dst.events, src.event].sort((a, b) => a.start - b.start);
    if (dst.name.trim().length > 0) {
      src.event.character = dst.name.trim();
    }
    return true;
  }

  /// 交换两条轨道在指定分段 [startSec, endSec) 内的 asr 事件归属：
  /// A 轨分段内的事件移到 B 轨，B 轨分段内的事件移到 A 轨。
  /// 仅作用于完全位于分段内的事件（时间/文本不变），character 跟随目标轨道名；
  /// 跨分段边界的事件不参与（避免长句被相邻分段来回移动）。
  /// 用于手动修正跨段说话人归属错误。
  /// 返回是否成功（两轨均为 asr 类型且至少一侧有分段内事件）。
  function swapTrackEventsInSegment(
    trackAId: string,
    trackBId: string,
    startSec: number,
    endSec: number
  ): boolean {
    if (!currentProject.value) return false;
    const ta = findTrack(trackAId);
    const tb = findTrack(trackBId);
    if (!ta || !tb || ta.id === tb.id) return false;
    if (ta.type !== "asr" || tb.type !== "asr") return false;

    // 完全包含于分段内的事件才交换
    const inSeg = (e: TimelineEvent) => e.start >= startSec && e.end <= endSec;
    const aIn = ta.events.filter(inSeg);
    const bIn = tb.events.filter(inSeg);
    if (aIn.length === 0 && bIn.length === 0) return false;

    recordSnapshot();
    ta.events = ta.events.filter((e) => !inSeg(e));
    tb.events = tb.events.filter((e) => !inSeg(e));
    // 互换：原 A 的事件进 B 轨、原 B 的事件进 A 轨，character 跟随轨道名
    const setChar = (e: TimelineEvent, name: string) => {
      if ((e.type === "asr" || e.type === "manual") && name.trim().length > 0) {
        e.character = name.trim();
      }
    };
    for (const e of aIn) setChar(e, tb.name);
    for (const e of bIn) setChar(e, ta.name);
    ta.events = [...ta.events, ...bIn].sort((x, y) => x.start - y.start);
    tb.events = [...tb.events, ...aIn].sort((x, y) => x.start - y.start);
    return true;
  }

  /// 重命名轨道：asr/manual 事件的 character 跟随轨道角色名（空名不改名）
  function renameTrack(trackId: string, name: string) {
    const track = currentProject.value?.tracks.find((t) => t.id === trackId);
    if (!track) return;
    const trimmed = name.trim();
    if (trimmed.length === 0) return;
    recordSnapshot();
    track.name = trimmed;
    for (const e of track.events) {
      if (e.type === "asr" || e.type === "manual") {
        e.character = trimmed;
      }
    }
  }

  /// 更新轨道内容属性（仅 asr 轨道消费）："streamer" 主播语音 | "game" 游戏内容
  function updateTrackRole(trackId: string, role: string) {
    const track = currentProject.value?.tracks.find((t) => t.id === trackId);
    if (!track || track.track_role === role) return;
    recordSnapshot();
    track.track_role = role;
  }

  // ── 字幕预览 ───────────────────────────────────────────
  /// 预览层总开关（会话级，不持久化）
  const subtitlePreviewOn = ref(true);

  /// 切换单轨预览显示（纯显示偏好：不进撤销快照，随项目保存）
  function toggleTrackPreview(trackId: string) {
    const track = currentProject.value?.tracks.find((t) => t.id === trackId);
    if (track) track.preview_visible = !track.preview_visible;
  }

  /// 删除事件（聚焦清理由 UI 层负责）
  function removeEvent(id: string) {
    if (!currentProject.value) return;
    for (const track of currentProject.value.tracks) {
      const idx = track.events.findIndex((e) => e.id === id);
      if (idx >= 0) {
        recordSnapshot();
        track.events.splice(idx, 1);
        return;
      }
    }
  }

  /// 删除轨道（聚焦清理由 UI 层负责）
  function removeTrack(id: string) {
    if (!currentProject.value) return;
    if (!currentProject.value.tracks.some((t) => t.id === id)) return;
    recordSnapshot();
    currentProject.value.tracks = currentProject.value.tracks.filter((t) => t.id !== id);
  }

  /// 上移/下移轨道（调整显示顺序）
  function moveTrack(id: string, dir: "up" | "down") {
    const tracks = currentProject.value?.tracks;
    if (!tracks) return;
    const i = tracks.findIndex((t) => t.id === id);
    const j = dir === "up" ? i - 1 : i + 1;
    if (i < 0 || j < 0 || j >= tracks.length) return;
    recordSnapshot();
    [tracks[i], tracks[j]] = [tracks[j], tracks[i]];
  }

  /// 源轨事件并入目标轨（按 start 排序），删除源轨；character 为事件级字段，随事件保留
  function mergeTrack(srcId: string, dstId: string) {
    const tracks = currentProject.value?.tracks;
    if (!tracks) return;
    const src = tracks.find((t) => t.id === srcId);
    const dst = tracks.find((t) => t.id === dstId);
    if (!src || !dst || src.id === dst.id) return;
    recordSnapshot();
    dst.events = [...dst.events, ...src.events].sort((a, b) => a.start - b.start);
    currentProject.value!.tracks = tracks.filter((t) => t.id !== srcId);
  }

  /// 合并两个同轨事件：时间取并集（end 取较晚）、文本按时间顺序拼接、
  /// 保留时间更早事件的 id 与属性，删除另一事件；返回保留的 id。
  /// 条件：同轨、均非 ocr_region；requireAdjacent 时还需按 start 排序相邻。
  /// 不满足返回 null
  function mergeTwo(idA: string, idB: string, requireAdjacent = true): string | null {
    const a = findEvent(idA);
    const b = findEvent(idB);
    if (!a || !b || a.track.id !== b.track.id) return null;
    const { track } = a;
    const ea = a.event;
    const eb = b.event;
    if (ea.type === "ocr_region" || eb.type === "ocr_region") return null;
    const sorted = [...track.events].sort((x, y) => x.start - y.start);
    const ia = sorted.findIndex((e) => e.id === ea.id);
    const ib = sorted.findIndex((e) => e.id === eb.id);
    if (ia < 0 || ib < 0) return null;
    if (requireAdjacent && Math.abs(ia - ib) !== 1) return null;

    recordSnapshot();
    const [first, second] = ea.start <= eb.start ? [ea, eb] : [eb, ea];
    first.end = Math.max(first.end, second.end);
    first.text = first.text + "\n" + second.text;
    track.events = track.events.filter((e) => e.id !== second.id);
    track.events.sort((x, y) => x.start - y.start);
    return first.id;
  }

  /// 合并事件与其同轨排序后紧随其后的下一个事件（M 键），
  /// 语义与 mergeTwo 一致（保留时间更早的事件）
  function mergeAdjacent(id: string): string | null {
    const found = findEvent(id);
    if (!found) return null;
    const { track, event } = found;
    if (event.type === "ocr_region") return null;
    const sorted = [...track.events].sort((a, b) => a.start - b.start);
    const idx = sorted.findIndex((e) => e.id === id);
    if (idx < 0 || idx >= sorted.length - 1) return null;
    return mergeTwo(id, sorted[idx + 1].id);
  }

  function closeProject() {
    // 关闭前落盘（在置空前触发保存）
    clearTimeout(saveTimer);
    saveNow();
    currentProject.value = null;
    currentVideoMeta.value = null;
    sourceVideoMeta.value = null;
    videoImportError.value = null;
    saveState.value = "saved";
    clearHistory();
  }

  return {
    currentProject,
    recentProjects,
    currentVideoMeta,
    sourceVideoMeta,
    isLoading,
    videoImportError,
    saveState,
    createProject,
    openProject,
    importVideo,
    importSourceVideo,
    ensureDefaultTrack,
    ensureCorpusRegionTrack,
    findEvent,
    findTrack,
    removeEvent,
    renameTrack,
    updateTrackRole,
    subtitlePreviewOn,
    toggleTrackPreview,
    moveEventToTrack,
    swapTrackEventsInSegment,
    removeTrack,
    moveTrack,
    mergeTrack,
    mergeTwo,
    mergeAdjacent,
    splitEvent,
    updateOcrRegion,
    addOcrRegionEvent,
    updateEventText,
    updateEventTime,
    recordSnapshot,
    undo,
    redo,
    clearHistory,
    canUndo,
    canRedo,
    refreshRecentProjects,
    saveNow,
    ocrRunning,
    ocrProgress,
    ocrMessage,
    runOcr,
    writeOcrToCorpus,
    addCorpusItem,
    removeCorpusItem,
    corpusReady,
    timelineReady,
    asrRunning,
    asrProgress,
    asrMessage,
    runAsr,
    getAsrEngines,
    cancelAsr,
    llmRunning,
    llmProgress,
    llmMessage,
    runLlm,
    checkLlmRuntime,
    fuseRunning,
    runFuse,
    closeProject,
  };
});
