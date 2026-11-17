import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import tailwindcss from "@tailwindcss/vite";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [vue(), tailwindcss()],

  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: [
        "**/src-tauri/**",
        // 编辑器/代理的原子写会在目标目录里建 .<name>.<pid>.<uuid>.tmpdir/：
        // 监听器一碰到被锁的临时文件就 EBUSY 抛错，整个 dev server 会退出，这里一律忽略。
        /(^|[\\/])\.[^\\/]*\.tmpdir([\\/]|$)/,
      ],
    },
  },
}));
