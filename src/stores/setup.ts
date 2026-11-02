import { defineStore } from "pinia";
import { computed, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { RuntimeStatus } from "../types";
import { useProjectStore } from "./project";

/// 启动门控的阶段
/// - checking：正在判定（一次 invoke，走 .installed.json 快路径）
/// - needs：缺必需项，等开跑（首启会自动开跑，取消/跳过后回到这里）
/// - installing：安装子进程在跑（进度来自 runtime-install-progress 事件）
/// - failed：失败，可重试或受限进入
/// - done：刚装完，短暂停留后放行
/// - ready：放行（含"已就绪的老用户"与"跳过"两种来源）
export type SetupStage = "checking" | "needs" | "installing" | "failed" | "done" | "ready";

export const useSetupStore = defineStore("setup", () => {
  const stage = ref<SetupStage>("checking");
  const status = ref<RuntimeStatus | null>(null);
  const error = ref("");
  /// 用户选择了"先跳过（受限模式）"：主界面照常进，但欢迎页会挂一条横幅给回头的入口
  const skipped = ref(false);

  /// 主界面能不能渲染
  const passable = computed(() => stage.value === "ready" || stage.value === "done");

  /// 缺的必需项的中文名（失败文案用）
  const missingLabels = computed(() => {
    const items = status.value?.items ?? [];
    return (status.value?.missing_required ?? []).map(
      (id) => items.find((i) => i.id === id)?.label ?? id,
    );
  });

  async function refresh(): Promise<void> {
    try {
      const s = await invoke<RuntimeStatus>("runtime_status");
      status.value = s;
      if (s.error) {
        error.value = s.error;
        stage.value = "failed";
        return;
      }
      if (s.ready || s.skip) {
        error.value = "";
        stage.value = "ready";
        return;
      }
      // 安装进行中不要被打断回 needs
      if (stage.value !== "installing") stage.value = "needs";
    } catch (e) {
      error.value = String(e);
      stage.value = "failed";
    }
  }

  /// 跑一次安装（首启自动调用；失败后"重试"也走这里）
  async function install(force = false): Promise<void> {
    const project = useProjectStore();
    stage.value = "installing";
    error.value = "";
    try {
      const summary = await project.startRuntimeInstall("auto", force);
      if (summary.cancelled) {
        stage.value = "needs";
        return;
      }
      if (!summary.ok) {
        error.value = summary.message;
        stage.value = "failed";
        return;
      }
      // 装完立刻重建进程内 provider（否则要重启才生效），再复核一遍
      await project.reloadRuntimes().catch(() => null);
      await refresh();
      // refresh() 会改 stage，但 TS 按赋值把这里窄化成 "installing"，所以显式取回宽类型
      const now = stage.value as SetupStage;
      if (now === "ready") {
        stage.value = "done";
        return;
      }
      if (now !== "failed") {
        error.value = missingLabels.value.length
          ? `运行时已安装，但仍有必需项未就绪：${missingLabels.value.join("、")}`
          : "运行时已安装，但仍未就绪";
        stage.value = "failed";
      }
    } catch (e) {
      error.value = String(e);
      stage.value = "failed";
    }
  }

  /// 先跳过：进主界面，但记住"是受限进入"
  function skip(): void {
    skipped.value = true;
    stage.value = "ready";
  }

  /// 从欢迎页横幅回到准备页（不自动开跑，让用户自己点）
  function reopen(): void {
    skipped.value = false;
    error.value = "";
    stage.value = "needs";
  }

  return {
    stage,
    status,
    error,
    skipped,
    passable,
    missingLabels,
    refresh,
    install,
    skip,
    reopen,
  };
});
