import { computed, onMounted, onUnmounted, ref } from "vue";
import { useProjectStore } from "../stores/project";

/// 视频画面在容器内的真实渲染矩形（object-fit: contain 的 letterbox 校正）。
/// 供预览覆盖层（OCR 选区、字幕层等）把归一化坐标换算成容器像素。
/// 用法：模板根元素绑定 `ref="containerRef"`，读取 `contentRect`。
export function useVideoContentRect() {
  const projectStore = useProjectStore();

  const containerRef = ref<HTMLElement | null>(null);
  const containerSize = ref({ width: 0, height: 0 });
  let resizeObserver: ResizeObserver | null = null;

  onMounted(() => {
    const el = containerRef.value;
    if (!el) return;
    const update = () => {
      const r = el.getBoundingClientRect();
      containerSize.value = { width: r.width, height: r.height };
    };
    update();
    resizeObserver = new ResizeObserver(update);
    resizeObserver.observe(el);
  });

  onUnmounted(() => {
    resizeObserver?.disconnect();
  });

  /// 视频画面实际占据的矩形（left/top/width/height，容器像素）
  const contentRect = computed(() => {
    const cw = containerSize.value.width;
    const ch = containerSize.value.height;
    if (cw <= 0 || ch <= 0) return { left: 0, top: 0, width: cw, height: ch };
    const meta = projectStore.currentVideoMeta;
    if (!meta || meta.width <= 0 || meta.height <= 0) {
      return { left: 0, top: 0, width: cw, height: ch };
    }
    const scale = Math.min(cw / meta.width, ch / meta.height);
    const w = meta.width * scale;
    const h = meta.height * scale;
    return { left: (cw - w) / 2, top: (ch - h) / 2, width: w, height: h };
  });

  return { containerRef, contentRect };
}
