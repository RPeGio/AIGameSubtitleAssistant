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

  /// 请求序号：`load` 与 `patch` 都会刷新快照，而 `runtime_settings` 要读盘 + 起 powershell 查空间，
  /// 慢请求可能后到。没用序号时，一个慢的 load 会把刚 patch 出来的新快照整个覆盖掉
  /// （面板上表现为「刚改的值又跳回旧值」）。每次发起请求自增，回来时序号不是最新就丢弃（2026-11-16 审查 P2-8）。
  let seq = 0;

  async function load() {
    const mine = ++seq;
    loading.value = true;
    error.value = null;
    try {
      const next = await invoke<RuntimeSettingsSnapshot>("runtime_settings");
      if (mine !== seq) return; // 期间有更新的请求发出，本次结果作废
      snapshot.value = next;
    } catch (e) {
      if (mine !== seq) return;
      error.value = String(e);
      console.error("读取运行时设置失败:", e);
    } finally {
      if (mine === seq) loading.value = false;
    }
  }

  /// 只提交改动的字段；返回 false 表示后端拒绝了本次修改
  async function patch(p: RuntimeSettingsPatch): Promise<boolean> {
    const mine = ++seq; // 作废任何还在飞的 load：patch 的返回值才是最新真相
    saving.value = true;
    error.value = null;
    try {
      const next = await invoke<RuntimeSettingsSnapshot>(
        "update_runtime_settings",
        { patch: p }
      );
      if (mine === seq) snapshot.value = next;
      return true;
    } catch (e) {
      if (mine === seq) error.value = String(e);
      console.error("保存运行时设置失败:", e);
      return false;
    } finally {
      if (mine === seq) saving.value = false;
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
