// 把仓库里的运行脚本同步进 `src-tauri/resources/scripts/`，让它们随安装包一起发出去。
//
// 为什么需要这一步：一键安装运行时（欢迎页自检卡的"安装/更新运行时"按钮）由 Rust
// 侧拉起 `scripts/bootstrap_runtime.ps1`。开发期脚本就在仓库里，但**打包后的 exe
// 找不到它**——真机反馈 2026-10-29：用 MSI 装完点"安装运行时"直接失败（报
// "找不到安装脚本 scripts/bootstrap_runtime.ps1"）。
//
// 做法：保持 `scripts/` 是唯一源（不复制一份进仓库、不给 drift 机会），构建时把
// 需要的脚本拷进 resources 目录，`.gitignore` 忽略该目录 ⇒ 仓库里只有一份。
// Tauri 的 `bundle.resources` 用数组形式会保留目录结构，最终落在
// `<exe目录>/resources/scripts/bootstrap_runtime.ps1`，正是 runtime_install.rs
// 的候选路径之一。
import { copyFileSync, mkdirSync, existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const dest = join(root, "src-tauri", "resources", "scripts");

// 随包发布的脚本清单（新增请在此登记，并在 runtime_install.rs 的候选路径里对得上）
const FILES = ["bootstrap_runtime.ps1"];

mkdirSync(dest, { recursive: true });
for (const name of FILES) {
  const from = join(root, "scripts", name);
  if (!existsSync(from)) {
    console.error(`[sync_bundle_resources] 缺少源脚本: ${from}`);
    process.exit(1);
  }
  copyFileSync(from, join(dest, name));
}
console.log(`[sync_bundle_resources] 已同步 ${FILES.length} 个脚本 → src-tauri/resources/scripts/`);
