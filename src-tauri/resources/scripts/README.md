# resources/scripts/

打包时随安装包发布的运行脚本目录。

**这里的 `.ps1` 是构建产物，不要手改**：源文件永远是仓库根目录的 `scripts/`。
`pnpm build`（以及 `pnpm tauri build` 的前置步骤）会先跑
`scripts/sync_bundle_resources.mjs`，把需要的脚本拷到此处，再由
`src-tauri/tauri.conf.json` 的 `bundle.resources` 带进安装包。

当前随包发布的是 `bootstrap_runtime.ps1`（欢迎页"安装/更新运行时"按钮背后跑的
安装器）。打包后它落在 `<exe目录>/resources/scripts/bootstrap_runtime.ps1`——
Windows 上 `resource_dir()` 就是 exe 所在目录，正是
`src-tauri/src/ai_runtime/runtime_install.rs` 里 `bootstrap_candidates()` 的候选路径。

这个 README 必须保留：`bundle.resources` 用了 `resources/scripts/*` 通配，
目录里一个文件都没有时 Tauri 构建会直接失败。
