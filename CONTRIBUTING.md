# 贡献指南

[English](CONTRIBUTING_EN.md) | **中文**

感谢关注 GameSubtitleAssistant！本项目处于早期阶段，欢迎 issue、讨论和 PR。

## 环境搭建

先按 [README 的快速开始](README.md#快速开始)完成依赖安装与 AI 运行环境搭建，再阅读 [开发文档](docs/development.md)了解架构。注意：跑通集成测试需要 runtime 就绪（模型与二进制在位）。

## 开发流程

1. 从 `dev` 或 `master` 拉出功能分支，命名参考现有分支：
   - `feat/<主题>`：新功能（如 `feat/asr`、`feat/workflow`）
   - `fix/<主题>`：修复（如 `fix/ocr`）
   - 也可用 `docs/`、`style/`、`refactor/` 等前缀
2. 小步提交，commit message 遵循 **Conventional Commits**（`<type>(<scope>): <subject>`），仓库历史即示例：
   - `feat(ocr): merge 多数投票 + 空帧容错 + 孤立短段并入`
   - `fix(timeline): delete key on injected-store timelines + drag follow on auto-scroll`
   - `style(editor): compact row layout to widen text editing area`
   - `refactor(workflow): reuse review-pane timeline for corpus mini timeline`
3. 提交前验证：
   ```powershell
   pnpm build              # vue-tsc 类型检查 + vite build
   cd src-tauri; cargo test --release
   ```
4. 发起 Pull Request（目标分支一般为 `dev`），说明改动动机与验证方式。

## 代码约定

- **Rust**：核心逻辑 / 架构决策用中文注释，说明"为什么"而非"做了什么"；错误信息面向用户可读。
- **TypeScript / Vue**：保持简洁，不加解释性废话注释；`src/types/index.ts` 与 Rust 数据结构镜像，新增字段两侧同步。
- **外科手术式修改**：只改任务相关的代码；发现无关问题提 issue 而非顺手改。
- 前后端交互的新增命令 / 事件，记得同步更新[开发文档](docs/development.md)的命令清单与事件表。

## 报告问题

提 issue 时请附上：

- 应用版本与 Windows 版本
- 出问题的环节（语料 OCR / ASR / 融合 / 编辑导出）与所用的 AI 模块配置
- `check_*` 就绪探测的结果（应用内可见）与相关日志
- 若涉及识别质量：一小段可复现的视频或截图（注意脱敏）

## 许可证与模型

提交代码即表示同意以 [MIT](LICENSE) 许可你的贡献。请注意：`runtime/` 中下载的模型（尤其 Qwen2.5-3B 的 Qwen Research 许可）各自有使用条款，与本项目代码许可无关，详见 [THIRD_PARTY.md](THIRD_PARTY.md)。
