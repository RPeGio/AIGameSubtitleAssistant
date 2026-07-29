// ─── AI 运行时模块 ───────────────────────────────────────
// Phase 2-5 中逐步实现：
//
// AI Provider 接口定义:
//   trait TextProcessor { fn process(&self, input: String) -> Result<String>; }
//   trait OcrProvider { fn recognize(&self, image: &[u8]) -> Result<String>; }
//   trait AsrProvider { fn transcribe(&self, audio: &[u8]) -> Result<Vec<Segment>>; }
//
// 具体实现（后续 Phase 分别接入）:
//   - local_llama.rs: llama.cpp 封装，用于文本整理/纠错
//   - moss.rs: MOSS ASR + Speaker Diarization
//   - paddle_ocr.rs: PaddleOCR Python Worker 封装
//   - openai.rs: 可选云端 API 桥接
//   - ollama.rs: Ollama HTTP API 桥接
