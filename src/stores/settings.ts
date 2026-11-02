import { defineStore } from "pinia";
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type {
  MoveRuntimeResult,
  RuntimeSettingsPatch,
  RuntimeSettingsSnapshot,
} from "../types";
import {
  readAutosaveDelay,
  readAutosaveEnabled,
  readSnapDefault,
  setAutosaveDelay as persistAutosaveDelay,
  setAutosaveEnabled as persistAutosave,
  setSnapDefault as persistSnapDefault,
} from "../composables/appPrefs";

/// 设置面板的数据层：运行时侧走 Tauri 命令（落 runtime/config.json），
/// 界面偏好走 localStorage（与既有 gsa.* 键同一套）。
export const useSettingsStore = defineStore("settings", () => {
  const snapshot = ref<RuntimeSettingsSnapshot | null>(null);
  const loading = ref(false);
  const saving = ref(false);
  const error = ref<string | null>(null);

  /// 界面偏好（设置面板直接双向绑定）
  const autosave = ref(readAutosaveEnabled());
  const autosaveDelay = ref(readAutosaveDelay());
  const snapDefault = ref(readSnapDefault());

  async function load() {
    loading.value = true;
    error.value = null;
    try {
      snapshot.value = await invoke<RuntimeSettingsSnapshot>("runtime_settings");
    } catch (e) {
      error.value = String(e);
      console.error("读取运行时设置失败:", e);
    } finally {
      loading.value = false;
    }
  }

  /// 只提交改动的字段；返回 false 表示后端拒绝了本次修改
  async function patch(p: RuntimeSettingsPatch): Promise<boolean> {
    saving.value = true;
    error.value = null;
    try {
      snapshot.value = await invoke<RuntimeSettingsSnapshot>(
        "update_runtime_settings",
        { patch: p }
      );
      return true;
    } catch (e) {
      error.value = String(e);
      console.error("保存运行时设置失败:", e);
      return false;
    } finally {
      saving.value = false;
    }
  }

  async function moveRuntimeDir(target: string): Promise<MoveRuntimeResult> {
    saving.value = true;
    try {
      const res = await invoke<MoveRuntimeResult>("move_runtime_dir", { target });
      await load();
      return res;
    } finally {
      saving.value = false;
    }
  }

  async function deleteOldRuntimeDir(): Promise<string> {
    saving.value = true;
    try {
      const msg = await invoke<string>("delete_old_runtime_dir");
      await load();
      return msg;
    } finally {
      saving.value = false;
    }
  }

  async function openLogDir(): Promise<string> {
    return invoke<string>("open_log_dir");
  }

  // ── 界面偏好 ────────────────────────────────────────────
  function setAutosave(on: boolean) {
    autosave.value = on;
    persistAutosave(on);
  }

  function setAutosaveDelay(delay: number) {
    autosaveDelay.value = delay;
    persistAutosaveDelay(delay);
  }

  function setSnapDefault(on: boolean) {
    snapDefault.value = on;
    persistSnapDefault(on);
  }

  return {
    snapshot,
    loading,
    saving,
    error,
    autosave,
    autosaveDelay,
    snapDefault,
    load,
    patch,
    moveRuntimeDir,
    deleteOldRuntimeDir,
    openLogDir,
    setAutosave,
    setAutosaveDelay,
    setSnapDefault,
  };
});
