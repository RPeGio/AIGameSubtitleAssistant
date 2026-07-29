import { defineStore } from "pinia";
import { ref } from "vue";
import type { Project, RecentProject } from "../types";
import { invoke } from "@tauri-apps/api/core";

export const useProjectStore = defineStore("project", () => {
  const currentProject = ref<Project | null>(null);
  const recentProjects = ref<RecentProject[]>([]);
  const isLoading = ref(false);

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
      await refreshRecentProjects();
      return project;
    } finally {
      isLoading.value = false;
    }
  }

  async function refreshRecentProjects() {
    try {
      recentProjects.value = await invoke<RecentProject[]>("list_recent_projects");
    } catch (e) {
      recentProjects.value = [];
    }
  }

  function closeProject() {
    currentProject.value = null;
  }

  return {
    currentProject,
    recentProjects,
    isLoading,
    createProject,
    openProject,
    refreshRecentProjects,
    closeProject,
  };
});
