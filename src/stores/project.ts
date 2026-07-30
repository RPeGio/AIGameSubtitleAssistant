import { defineStore } from "pinia";
import { ref } from "vue";
import type { Project, RecentProject, VideoMetadata } from "../types";
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
          x1: 0.1,
          y1: 0.7,
          x2: 0.9,
          y2: 0.85,
        },
      ],
    });
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
    refreshRecentProjects,
    closeProject,
  };
});
