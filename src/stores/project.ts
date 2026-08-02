import { defineStore } from "pinia";
import { ref } from "vue";
import type {
  Project,
  RecentProject,
  VideoMetadata,
  TimelineEvent,
  Track,
} from "../types";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

function generateId(): string {
  return Date.now().toString(36) + Math.random().toString(36).slice(2, 8);
}

export const useProjectStore = defineStore("project", () => {
  const currentProject = ref<Project | null>(null);
  const recentProjects = ref<RecentProject[]>([]);
  const currentVideoMeta = ref<VideoMetadata | null>(null);
  const isLoading = ref(false);
  const videoImportError = ref<string | null>(null);

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
        currentProject.value = updated;
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
    const hasOcrRegion = currentProject.value.tracks.some(
      (t) => t.type === "ocr_region"
    );
    if (hasOcrRegion) return;

    currentProject.value.tracks.push({
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

  function closeProject() {
    currentProject.value = null;
    currentVideoMeta.value = null;
    videoImportError.value = null;
  }

  return {
    currentProject,
    recentProjects,
    currentVideoMeta,
    isLoading,
    videoImportError,
    createProject,
    openProject,
    importVideo,
    ensureDefaultTrack,
    findEvent,
    findTrack,
    splitEvent,
    updateOcrRegion,
    refreshRecentProjects,
    closeProject,
  };
});
