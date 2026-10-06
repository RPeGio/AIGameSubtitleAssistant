# CLAUDE.md

## 开发前必读

- `GameSubtitleAssistant_Plan.md`
- `IDENTITY.md` - 在接手项目前你需要明确的对话风格
### 选择性读取
- `session-ses_0528.md`（主分支 agent 对话记录）

Behavioral guidelines to reduce common LLM coding mistakes. Merge with project-specific instructions as needed.

**Tradeoff:** These guidelines bias toward caution over speed. For trivial tasks, use judgment.

## 1. Think Before Coding

**Don't assume. Don't hide confusion. Surface tradeoffs.**

Before implementing:
- State your assumptions explicitly. If uncertain, ask.
- If multiple interpretations exist, present them - don't pick silently.
- If a simpler approach exists, say so. Push back when warranted.
- If something is unclear, stop. Name what's confusing. Ask.

## 2. Simplicity First

**Minimum code that solves the problem. Nothing speculative.**

- No features beyond what was asked.
- No abstractions for single-use code.
- No "flexibility" or "configurability" that wasn't requested.
- No error handling for impossible scenarios.
- If you write 200 lines and it could be 50, rewrite it.

Ask yourself: "Would a senior engineer say this is overcomplicated?" If yes, simplify.

## 3. Surgical Changes

**Touch only what you must. Clean up only your own mess.**

When editing existing code:
- Don't "improve" adjacent code, comments, or formatting.
- Don't refactor things that aren't broken.
- Match existing style, even if you'd do it differently.
- If you notice unrelated dead code, mention it - don't delete it.

When your changes create orphans:
- Remove imports/variables/functions that YOUR changes made unused.
- Don't remove pre-existing dead code unless asked.

The test: Every changed line should trace directly to the user's request.

## 4. Goal-Driven Execution

**Define success criteria. Loop until verified.**

Transform tasks into verifiable goals:
- "Add validation" → "Write tests for invalid inputs, then make them pass"
- "Fix the bug" → "Write a test that reproduces it, then make it pass"
- "Refactor X" → "Ensure tests pass before and after"

For multi-step tasks, state a brief plan:
```
1. [Step] → verify: [check]
2. [Step] → verify: [check]
3. [Step] → verify: [check]
```

Strong success criteria let you loop independently. Weak criteria ("make it work") require constant clarification.

---

**These guidelines are working if:** fewer unnecessary changes in diffs, fewer rewrites due to overcomplication, and clarifying questions come before implementation rather than after mistakes.

---

## 5. Project-Specific Workflow

### Task Granularity
- Each Phase is split into small sub-tasks
- **One task per conversation round** — do not batch multiple tasks in one response
- Before coding each task, present the plan and let the user confirm

### 素材责任边界与异常上报（重要）

本项目的输入素材分两类，**归属不同，绝不能混淆**：

| 素材 | 谁负责正确 | 出现异常时 |
|---|---|---|
| **管线 raw 产出**（未经人工干预：OCR / ASR / 融合的原始产物） | **管线自己** | 噪音、碎片、不合理分段都是**管线缺陷**——就地定位根因、**在管线层修**，并登记缺陷台账 |
| **人工预校对后的素材**（用户逐条核对过） | **用户** | 发现异常就**直接、明确地告知用户**，不要自己绕开 |

**不要默认"用户做过的一定是对的"，然后自己瞎摸索绕路。** 判据只有一句话：

> **这段内容本应由谁保证正确？** 该由管线保证（raw 产出）→ 修管线；该由用户保证（校对后）→ 告知用户。

反面案例（本项目真实发生过）：在**人工预校对过**的素材里发现"同一句台词被拆成两段"，
直接当成既有事实、准备在**下游（融合侧）加一层合并**绕过——既没指出这其实是
**OCR 合并层的 raw 缺陷**，也没告知用户这是**校对时的遗漏**。

沉默绕过的代价是双重的：**掩盖管线缺陷**（真实根因没人修）＋**把用户的遗漏固化进后续结论**
（错误的前提被当作事实继续使用）。发现异常却犹豫"这算不算问题"时，宁可直说。

### Git Policy

- **Do NOT auto-commit** after completing a task
- After coding, output a **conventional commit message** (title + body) that the user can copy for manual commit
- The user is responsible for committing and pushing

### Code Style
- Rust: write comments in Chinese for core logic / architecture (the user is learning Rust)
- Frontend: keep TypeScript/Vue concise, no unnecessary comments
- Match existing project conventions
