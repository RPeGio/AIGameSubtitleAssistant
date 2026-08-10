import { defineStore } from "pinia";
import { computed, onScopeDispose, ref, watch } from "vue";
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
  LlmProgress,
  LlmRuntimeStatus,
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
  const undoStack = ref<Track[][]>([]);
  const redoStack = ref<Track[][]>([]);
  const MAX_HISTORY = 60;

  function snapshot(): Track[] {
    return JSON.parse(JSON.stringify(currentProject.value?.tracks ?? [])) as Track[];
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
    currentProject.value.tracks = undoStack.value.pop()!;
  }

  function redo() {
    if (!currentProject.value || redoStack.value.length === 0) return;
    undoStack.value.push(snapshot());
    currentProject.value.tracks = redoStack.value.pop()!;
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
    recordSnapshot();
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
    videoImportError.value = null;
    saveState.value = "saved";
    clearHistory();
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
    removeEvent,
    renameTrack,
    moveEventToTrack,
    removeTrack,
    moveTrack,
    mergeTrack,
    mergeTwo,
    mergeAdjacent,
    splitEvent,
    updateOcrRegion,
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
    asrRunning,
    asrProgress,
    asrMessage,
    runAsr,
    llmRunning,
    llmProgress,
    llmMessage,
    runLlm,
    checkLlmRuntime,
    closeProject,
  };
});
