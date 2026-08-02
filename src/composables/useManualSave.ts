import { useMessage } from "naive-ui";
import { useProjectStore } from "../stores/project";

/// 手动保存：立即落盘，成功时从上方飘出提示
export function useManualSave() {
  const message = useMessage();
  const projectStore = useProjectStore();

  async function manualSave() {
    const result = await projectStore.saveNow();
    if (result === "ok") {
      message.success("已保存");
    } else if (result === "failed") {
      message.error("保存失败");
    }
    // "skipped"（无项目或仍在加载中）：没有实际执行保存，静默不提示
  }

  return { manualSave };
}
