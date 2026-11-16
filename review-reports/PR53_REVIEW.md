# PR #53 审查记录

- **PR**：`feat(runtime): 资产改由 ModelScope 主源托管 + 下载器按实测速率选源`
- **分支**：`feat/runtime-distribution` → `feat/first-run-prep`（3 提交，2026-11-12 ~ 11-14）
- **审查日期**：2026-11-16
- **审查者**：15 年资深工程师视角，严重度映射 🔴→P0 / 🟡→P1 / 🟢→P2
- **审查范围**：`git diff 9cae08c 2a8306e`，8 文件（+358/−19）

## 审查范围

| 提交 | 内容 |
|---|---|
| `ef50510` | 下载器按实测速率选源（新增 `-ProbeMB` / `-ProbeSec` + `Measure-UrlSpeed` + `Invoke-SegmentedDownload` 重写） |
| `2a8306e` | 清单 7 条资产 `urls[]` 前插 ModelScope 直链（变三级：ModelScope → ESA → GitHub）+ 文档补记 |
| `b5eae3c` | Merge PR #51（与本 PR 的审查无关） |

`scripts/bootstrap_runtime.ps1` 是唯一有逻辑改动的文件（+101）；其余全是文档
（`RUNTIME_BOOTSTRAP_CONTRACT.md` +32、`engines_vulkan_design(_EN).md` +81/+134、
`development(_EN).md` +10/+10、`MISC_TODO.md` +2）与清单数据（`manifest.json` +7）。

## 总评

**原为 1 条 P0 阻塞合并 + 6 条 P1；P0-1、P1-1 ~ P1-3 已于 2026-11-16 在本 PR 内修复并复测
（见「处置记录」），现在剩余 3 条 P1（均为文档）建议本 PR 内处理。**

这是一个「只改数据 + 一个函数」的小 PR，方向我认同：把「固定候选顺序」换成「实测选源」，
恰好对应文档自己记下的那条事实——同一个源白天 8.9 MB/s、夜里 0.27 MB/s，固定顺序必然在一半
时间里选错。清单换源也确实只动了 `urls[]`，`sha256`/`size`/`dest` 一字未改，我逐条核对 7 条
资产的新直链：都能 206、`Content-Range` 总长与 `manifest.size` 精确相等、LFS 对象路径里编码的
sha256 与清单记录的 sha256 全部吻合——**换源没有换掉字节**，这点是本 PR 最值得肯定的部分。

但 `Invoke-SegmentedDownload` 的重写引入了一个**新的失败语义**：`$ranked`（交给单流兜底的那个
队列）只由「探针通过的源」+「镜像」组成，而 `$ranked.Count -eq 0` 的兜底又因为镜像永远存在而
不可达。结果是一旦 `$probed` 为空，兜底队列就只剩**第三方代理 gh-proxy 一条**，三条真实直链
（含本 PR 刚设为主源的 ModelScope）被静默踢出重试集合。这不是理论问题：`-SegmentMB 0` 与
`curl.exe 缺失`都是契约里写明的正式路径，而文档自己记录了「gh-proxy 0.13 MB/s、ModelScope
7.88 MB/s」的夜晚——那种网络下 ModelScope 通、gh-proxy 死，安装却必然报
`所有候选源都失败`。基线 `9cae08c` 在这两条路径上传的是 `$candidates`（全部候选），
**所以这是回归，不是既有缺陷**。

另一半问题在「可复现性」：契约 §4.5 是新功能的唯一权威描述，但它写的 curl 调用与代码不一致
（超时 6 vs 4、`-w` 两个字段 vs 四个、状态码门槛「非 2xx/206」vs「必须是 206」），而且没写
「吞吐扣掉了 TTFB」这个关键机制——照文档的命令复测，得到的数字必然与安装器选源的依据不同。
另外 `Sort-Object` 在 PS 5.1 上**不稳定**（我实测 n=3 就直接把三个等速源整体倒序），
而 `-ProbeMB 0` 分支正好喂给它一组全部 `Mbps=0` 的条目，于是「取第一个支持 Range 的候选」
这条文档承诺变成了「随机取一个」。

**与 PR #52 的合并风险（请务必先读）**：本 PR 的清单与选源逻辑要和 PR #52 的
`Get-UrlCandidates`（`-Source` 三档重写）合并。我在临时 worktree 里做过试合并：
**文本零冲突**，但合并结果里 `Get-UrlCandidates` 取的是 PR #52 那版，带着
`$mirror = New-Object System.Collections.ArrayList` 遮蔽 `-Mirror` 参数的 bug（PR #52 P0-1）。
合并后镜像候选永远为空，于是：① 本 PR 精心放在 `$ranked` 队尾的「镜像只做单流兜底」
（`scripts/bootstrap_runtime.ps1:678-680`）成为死代码；② 本 PR P0-1 的症状会从
「只剩镜像」变成「`$ranked` 真为空 → 回落到全部候选」——**症状变化但根因仍在**，
且「强制镜像」设置继续是空操作。两个 PR 谁先合都要处理这一处，建议 PR #52 的 P0-1 先修。

## 逐条发现

### 🔴 P0-1 `$probed` 为空时单流兜底只剩镜像一条，三条直链被静默丢弃 ✅ 已修复

> **状态**：✅ 2026-11-16 已在本 PR 内修复（见文末「处置记录」P0-1）。
> 下方保留发现时的原始取证（行号对应修复前），便于后人核对修复是否真的覆盖了这些路径。

- **位置**：`scripts/bootstrap_runtime.ps1:674-681`（构造 `$ranked`）与 `:692-694`（交给单流）
- **问题**：`$ranked` 只有两个来源——`:675` 遍历 `$probed`、`:679` 追加 `Kind -eq "mirror"`。
  一旦 `$probed` 为空，`$ranked` 就只含镜像那一条；于是 `:681` 的
  `if ($ranked.Count -eq 0) { $ranked = @($candidates) }` **永远不成立**（`-Mirror` 默认非空、
  清单每条都有 `url`，镜像必然被追加），那句「兜底用全部候选」是死代码。
  `:692-694` 把这份单元素队列交给 `Invoke-SingleStream`，ModelScope / ESA / GitHub 三条直链
  一次都不会被尝试；日志里它们只留下一行「跳过 …（不支持 Range，或探测失败）」，
  用户看不出它们已被踢出重试队列。
  `$probed` 为空的三个触发路径（都在正式支持范围内）：

  | 触发 | 依据 |
  |---|---|
  | `-SegmentMB 0`（关闭分段） | `:644` 的 `if ($SegmentMB -le 0)` 分支**不做任何探测**，`$probed` 保持空 |
  | `curl.exe` 缺失 | `:629-632` 把 `$SegmentMB` 改成 0，落进同一条路径；契约写明这是 Win10 <1803 的支持场景 |
  | 全部探针失败 | 三条直链 TTFB 均 > `-ProbeSec`(4 s) 或 4 s 内零字节（`:459` 判 `ok=false`） |

- **实测复现**（用从 `:397-412` / `:643-681` 逐字抽出的函数体，真实 `powershell.exe` 5.1.26100.9818）：

  | 输入 | 候选总数 | `$ranked` 实际内容 |
  |---|---|---|
  | 默认（`SegmentMB 64`，探针全过） | 4 | GitHub → ESA → ModelScope → mirror |
  | `-SegmentMB 0`，`-Mirror` 默认 | 4 | **mirror（1 条）** |
  | `-SegmentMB 0`，`-Mirror ""` | 3 | ModelScope → ESA → GitHub（3 条，正常） |
  | 探针全失败，`-Mirror` 默认 | 4 | **mirror（1 条）** |

  即：**关掉 `-Mirror` 反而比默认更安全**——这是最直观的荒谬之处。
- **为什么致命**：本 PR 自己的文档记着「2026-11-13 夜间 ModelScope 7.88 MB/s｜GitHub 0.13｜
  gh-proxy 0.13｜ESA 0.09」。那一夜 gh-proxy 是死的、ModelScope 是通的；而本 PR 恰好把
  ModelScope 设为主源——却在 `-SegmentMB 0` / curl 缺失路径上把它从重试队列里删了。
  用户看到的是 `所有候选源都失败：…`，指着一个与真实原因无关的代理。基线 `9cae08c`
  在 `:623` / `:686` 传的都是 `$candidates`（全部候选），所以**这是本 PR 引入的回归**。
- **修复建议**：兜底判据要看「探针结果」而不是「排序结果」，并且镜像必须在兜底**之后**追加：
  先按探针成功的源排序，`$probed` 为空时用**全部非镜像候选**打底，最后再补镜像。
- **修复后代码示例**：

  ```powershell
    # 排序队列：测速过的源（快→慢）→ 未测到速率的直链（保序）→ 镜像（只做单流兜底）
    $ranked = @()
    foreach ($p in ($probed | Sort-Object Mbps -Descending)) {
      $ranked += [pscustomobject]@{ Url = $p.Url; Kind = $p.Kind; Length = $p.Length }
    }
    # 探针一个都没过（-SegmentMB 0 / curl 缺失 / 全部超时）⇒ 直链仍是合法的单流源，不能丢
    if ($ranked.Count -eq 0) {
      foreach ($c in $candidates) {
        if ($c.Kind -ne "mirror") { $ranked += [pscustomobject]@{ Url = $c.Url; Kind = $c.Kind; Length = [int64]0 } }
      }
    }
    # 镜像永远排最后（分段会整包重取，只适合单流兜底）
    foreach ($c in $candidates) {
      if ($c.Kind -eq "mirror") { $ranked += [pscustomobject]@{ Url = $c.Url; Kind = $c.Kind; Length = [int64]0 } }
    }
    if ($ranked.Count -eq 0) { $ranked = @($candidates) }   # 清单只有镜像的极端情况
  ```

  同时把 `:681` 的注释从「兜底」改掉——它现在真的不可达，留着会误导下一个人。

### 🟡 P1-1 `Sort-Object` 在 PS 5.1 上不稳定，`-ProbeMB 0` 的「按候选顺序」承诺变成随机 ✅ 已修复

- **位置**：`scripts/bootstrap_runtime.ps1:652-657`（注释承诺「沿用旧行为」）+ `:675`、`:667`（排序）
- **问题**：`-ProbeMB 0` 分支把每个通过 `Test-RangeSupport` 的候选都以 `Mbps = 0.0` 放进
  `$probed`（`:655`），随后 `:675` 用 `Sort-Object Mbps -Descending` 排序。**等键时
  PowerShell 5.1 的 `Sort-Object` 不保证保序**，实测把三个等速项整体倒序。
  文档与代码注释都承诺「回到『第一个支持 Range 的候选优先』」
  （`docs/RUNTIME_BOOTSTRAP_CONTRACT.md:329`、`scripts/bootstrap_runtime.ps1:653`），实际不成立。
  更关键的是**默认路径也会踩到**：本 PR 记录的夜间数据里 GitHub 与 gh-proxy 都是 `0.13 MB/s`，
  两源等速即触发不稳定排序；而「等速」恰恰是最常见的真实情形。
- **实测**（PS 5.1.26100.9818，输入为真实候选顺序 ModelScope → ESA → GitHub，均 `Mbps=0.0`）：

  | n（等键条目） | 排序结果 | 保序 |
  |---|---|---|
  | 3（真实用例） | GitHub → ESA → ModelScope（**整体倒序**） | ✗ |
  | 4 | site2 → site3 → site0 → site1 | ✗ |
  | 5 | site3 → site4 → site2 → site0 → site1 | ✗ |
  | 8 | site5 → site4 → site7 → site6 → site1 → site0 → site3 → site2 | ✗ |
  | 不等键（9.0 / 5.0 / 1.5） | ESA → GitHub → ModelScope | ✓ |

  被测值三维确定（同一输入重复跑结果一致），所以不是「随机」，而是**每次都稳定地选错**。
  按文档自己的实测，这一错是 `0.13` vs `7.88 MB/s`（60 倍）；`moss-cuda` 651 MB 从 ~1.5 分钟
  变成 ~80 分钟。
- **修复建议**：`-ProbeMB 0` 分支根本不该进排序（保持插入序）；需要按速率排序时用
  「索引 + 速率」显式排序，别依赖 `Sort-Object` 的稳定性。
  ⚠ **注意：`-Stable` 参数在 PS 5.1 不存在**（实测报
  `A parameter cannot be found that matches parameter name 'Stable'`），不要在只支持 5.1 的脚本里用它。
- **修复后代码示例**：

  ```powershell
    # -ProbeMB 0 走「按候选顺序取第一个支持 Range 的源」，不能进速率排序（等键排序不稳定）
    $ranked = @()
    if ($ProbeMB -le 0 -or $SegmentMB -le 0) {
      foreach ($p in $probed) { $ranked += [pscustomobject]@{ Url = $p.Url; Kind = $p.Kind; Length = $p.Length } }
    } else {
      # 显式带上原始序号做次键，避免等速时顺序被重排（PS 5.1 的 Sort-Object 不稳定，也没有 -Stable）
      $i = 0
      $keyed = foreach ($p in $probed) {
        [pscustomobject]@{ Seq = $i; Url = $p.Url; Kind = $p.Kind; Length = $p.Length; Mbps = $p.Mbps }
        $i++
      }
      foreach ($p in ($keyed | Sort-Object @{ Expression = "Mbps"; Descending = $true }, @{ Expression = "Seq"; Descending = $false })) {
        $ranked += [pscustomobject]@{ Url = $p.Url; Kind = $p.Kind; Length = $p.Length }
      }
    }
  ```

### 🟡 P1-2 探针失败被当成「源不可用」，可用的源被永久踢出兜底队列 ✅ 已修复

- **位置**：`scripts/bootstrap_runtime.ps1:659-664`（失败只 `Log`，不进 `$probed`；而 `$probed` 是 `$ranked` 的唯一来源）
- **问题**：探针失败有两种完全不同的含义，代码把它们混为一谈：
  - **「确实不可用」**——返回 200/404/500、连接被拒、DNS 失败。丢弃合理。
  - **「没测出来」**——TTFB 超过 `-ProbeSec`(4 s)、4 s 内零字节、中途被 `--max-time` 掐断。
    这些源**仍可能正常单流下载**，而现在它们从重试集合里彻底消失。

  而「源慢到不能分段」恰恰是最需要单流兜底的情形——兜底路径本来就是为「Range 走不通」设计的，
  现在却因为探针超时把这条源从兜底里删了。叠加 P0-1，重试集合可从 4 条缩到 1 条。
  实测两例（本地构造服务，`Measure-UrlSpeed <url> 2 4`）：
  - 限速 64 KiB/s（仍在传数据）：`ok=True, mbps=0.062` ⇒ 保住名单，但被排到镜像之后；
  - TTFB 6 s：烧掉整 4 s 后 `ok=False` ⇒ **从兜底队列消失**。
- **修复建议**：区分「有意拒绝」与「没测出来」。后者保留在候选里、标 `Slow`、排到镜像之前。
- **修复后代码示例**：

  ```powershell
        $m = Measure-UrlSpeed $c.Url $ProbeMB $ProbeSec
        if ($m.ok) {
          $probed += [pscustomobject]@{ Url = $c.Url; Kind = $c.Kind; Mbps = $m.mbps; Length = $m.length; Slow = $false }
        } elseif (-not $m.rejected) {
          # 超时 / 零字节：源可能只是慢，不能从兜底里删掉，降级到队尾
          Log "未测到速率 $($c.Url)（保留为单流兜底）"
          $probed += [pscustomobject]@{ Url = $c.Url; Kind = $c.Kind; Mbps = 0.0; Length = [int64]0; Slow = $true }
        } else {
          Log "跳过 $($c.Url)（HTTP 状态表明源不可用）"
        }
  ```

  `Measure-UrlSpeed` 相应地在返回值里带 `rejected`（`$code` 是 200/4xx/5xx 时为 true，
  `$code -eq 0` 的超时/连接失败为 false）。

### 🟡 P1-3 `Measure-UrlSpeed` 会把「无 `Content-Range` 的 206」「只回几百字节的 206」判成高速源 ✅ 已修复

- **位置**：`scripts/bootstrap_runtime.ps1:452-468`（判据只有 `:459` 的 `$code -ne 206 -or $size -le 0`）
- **问题**：健康的唯一判据是 `http_code = 206 且 size > 0`，而 `mbps` 直接由
  `$size / ($total - $ttfb)` 得出（`:465-468`）。于是「回得少、回得快」会被算成「极快」。
  实测（本地构造响应，`Measure-UrlSpeed <url> 2 4`）：

  | 响应形态 | ok | mbps | length |
  |---|---|---|---|
  | 正常 206 | True | 74.98 | 8388608 |
  | **206 但无 `Content-Range`** | **True** | **897.263** | **0** |
  | **206 只回 1 KiB（请求 2 MiB）** | **True** | **0.705** | **0** |
  | 206 但无视 Range 发整包 | True | 81.01 | 8388608 |
  | 200 整包 | False | 0 | 0 |

  另有一个相关但独立的匹配缺陷：`:463` 的正则把请求上界写死成 `0-$last`，
  任何「服务端按自己上限裁短」或「对象比探针小」的合法 206 都匹配不上（实测
  `Content-Range: bytes 0-1121691/1121692` 不匹配、`bytes 0-1023/8388608` 不匹配），
  `$len` 保持 0。`$len=0` 本身被 `:686` 的 `-gt 0` 挡住（回落到清单 `size`，运气好），
  但 `mbps` 已经被算错。
- **影响**：一个会裁短 Range 的 CDN 会以 897 MB/s 拿下 `$rangeUrl`，随后
  `Invoke-CurlSegment` 每段都 `SEGMENT_SHORT`（`:547`）→ 重试 3 次 → 整段回退单流。
  用户在 651 MB 的 `moss-cuda` 上白等三轮重试。若清单某条没有 `size`，`$len=0`
  还会让分段整条退化。
- **修复建议**：把「测到速率」的门槛补齐——要求 `Content-Range` 命中、且实收字节达到
  请求量的一半；正则放宽成不写死上界。
- **修复后代码示例**：

  ```powershell
      if ($code -ne 206 -or $size -le 0) { return @{ ok = $false; mbps = 0.0; length = [int64]0; rejected = ($code -ne 0) } }
      $len = [int64]0
      try {
        $h = [System.IO.File]::ReadAllText($hdr)
        # 不写死上界：服务端可以按自己的上限裁短，也可以整包返回（对象比探针小）
        if ($h -match "(?i)Content-Range:\s*bytes\s+0-\d+/(\d+)") { $len = [int64]$Matches[1] }
      } catch { }
      # 没拿到总长、或实收不足请求量一半 ⇒ 这不是一次可信的测速，不能参与选源
      $want = [math]::Min([int64]$last + 1, $(if ($len -gt 0) { $len } else { [int64]$last + 1 }))
      if ($size -lt ($want * 0.5)) { return @{ ok = $false; mbps = 0.0; length = $len; rejected = $false } }
  ```

### 🟡 P1-4 契约 §4.5 对新功能的探针描述与实现四处不符，且 `-ProbeSec` 全无文档

- **位置**：`docs/RUNTIME_BOOTSTRAP_CONTRACT.md:326`（对照 `scripts/bootstrap_runtime.ps1:452`、`:459`、`:465-468`、`:68`）
- **问题**：契约 §4.5 是这个新功能唯一的权威描述，但它写的 curl 调用**每一栏**都与代码不同：

  | 项 | 文档 `:326` | 代码 |
  |---|---|---|
  | 超时 | `--max-time 6`（固定 6） | `--max-time $timeoutSec`，默认来自 `[int]$ProbeSec = 4`（`:68`） |
  | `-w` 格式 | `'%{http_code} %{speed_download}'`（2 字段） | `"%{http_code} %{size_download} %{time_total} %{time_starttransfer}"`（4 字段）+ `-D $hdr` 抓头，代码**从不使用** `speed_download` |
  | 可用状态码 | 「`http_code` 非 2xx/206 视为不可用」（读起来像 2xx 或 206 都行） | `:459` `if ($code -ne 206 -or $size -le 0)` ⇒ **只接受 206**，回 200 的镜像被判不可用 |
  | 吞吐公式 | 未提 | `:465-468` `$xfer = $total - $ttfb; mbps = $size / $xfer / 1MB`——**故意扣掉 TTFB**（`:439-445` 注释：含 TTFB 会把 8.9 MB/s 算成 1.4 MB/s） |

  最后一行是实质性的：照着文档的命令复测，得到的 `mbps` 与安装器选源的依据**系统性地不同**，
  而 TTFB 扣除正是本 PR 最核心的一个测量决定。
  另外 `-ProbeSec`（`scripts/bootstrap_runtime.ps1:68`，脚本头 `:48` 自述「单源测速请求超时（秒），默认 4」）
  在 `docs/` 下**零引用**——参数表停在 `:52` 没有它的行，§4.5 标题也只写 `-ProbeMB`。
  可它恰好是「慢但能通的源算不算可用」的那个旋钮，却无法从契约里发现。
- **修复建议**：把 `:326` 整行换成真实命令与真实门槛（含 TTFB 扣除与 `Content-Range` 复用），
  并在 `:48` 后补 `-ProbeSec` 一行。
- **修复后代码示例**（文档片段）：

  ```markdown
  | 探针 | `Measure-UrlSpeed(url, ProbeMB, ProbeSec)`：`curl -s -L --max-time <ProbeSec，默认 4> -r "0-(<ProbeMB，默认 2>MB-1)" -D <临时头文件> -o NUL -w "%{http_code} %{size_download} %{time_total} %{time_starttransfer}"`，**只接受 `http_code = 206` 且实收字节 > 0**；吞吐 = `size_download / (time_total − time_starttransfer)`（扣掉 TTFB，否则 2 MB 探针会系统性低估）；总长从临时头文件的 `Content-Range` 解析，解析不到就回落到清单 `size` |
  ```

### 🟡 P1-5 清单字段表写着「`urls` 顺序即优先级」，被本 PR 推翻，而同一文件 177 行后写的是相反的话

- **位置**：`docs/RUNTIME_BOOTSTRAP_CONTRACT.md:172` 对照 `:349`；代码 `scripts/bootstrap_runtime.ps1:675`
- **问题**：`:172` 是**清单作者最先读的字段参考表**，它说 `urls` 是「多源候选（字符串数组，**顺序即优先级**）」。
  但本 PR 之后正常路径是按实测速率排序（`:675`），顺序只在「探针不可用」时才有意义——
  这正是同一文件 `:349` 写的话：「顺序只是『探针不可用时的缺省偏好』」。
  两处相隔 177 行、结论相反，代码站 `:349` 那边。
- **影响**：给清单加新镜像的人会认为「放在第 0 位就是首选」，实际只在
  `-ProbeMB 0` / curl 缺失 / `-SegmentMB 0` 或速率恰好相等时才生效——而后者（等速）
  又会踩 P1-1 的排序不稳定。这条错误认知会直接导向错误的运维动作。
- **修复建议**：改 `:172` 的措辞，并指向 §4.5。
- **修复后代码示例**：

  ```markdown
  | `urls` | ✖ | 多源候选（字符串数组）。**顺序只在「探针不可用」时作为缺省偏好**——正常路径按 §4.5 的实测吞吐排序，顺序不生效。存在时排在 `-Mirror` 拼接结果与 `url` 之前。Rust 侧不消费该字段（`parse_manifest_items` 只取 `id/flavor/sha256/size/unpacked_size`），它是**下载器的输入** |
  ```

### 🟡 P1-6 设计文档仍写「ModelScope 已调研未落地」，而本 PR 已把它设为主源

- **位置**：`docs/engines_vulkan_design.md:405-406`（EN 对应 `docs/engines_vulkan_design_EN.md:411-412`）
- **问题**：原文是

  > **仍未做（留档）**：… 外网加速（ModelScope 主源 / 自建 CF Worker 反代）已调研未落地，见 §8.11 的调研结论。

  但同一文件 `:487` 就是本节新增的 `### 8.14 资产迁到 ModelScope 魔搭托管（2026-11-14）`，
  `:492` 写「放弃 ESA 主源，改为 ModelScope 托管」，清单 7 条 `urls[0]` 全是 ModelScope 直链。
  而且 `:406` 把读者指向「§8.11 的调研结论」，但 §8.11 的结尾正是这条过期 bullet 本身
  （`:397-406`），答案在 80 行之后的 §8.14。这是本 PR 新增文件里**唯一一条彻底为假**的断言
  （其余只是过期），也是唯一会让人重复做一次调研的断言。
- **修复建议**：改成「已落地 + 未做部分」，并把指针指到 §8.14。
- **修复后代码示例**：

  ```markdown
  - **仍未做（留档）**：OCR 的安装门禁 UI（未装 OCR 时不许进项目）属前端接线，与 11c 一并做；
    外网加速：**ModelScope 主源已落地**（2026-11-14 起为 `urls[0]`，见 §8.14），
    自建 CF Worker 反代仍未做。
  ```

### 🟢 P2-1 探针成本与目标尺寸脱钩：7 条目白下 ~42 MB，最小条目探针量是自身 5.5 倍

- **位置**：`scripts/bootstrap_runtime.ps1:647-665`（串行 foreach + 每候选 2 MB）、`:452` 的 `-o NUL`
- **问题**：每个条目 3 条非镜像候选 × 2 MB = 6 MB，7 条目 **≈ 42 MB 探针流量全部丢弃**
  （`-o NUL`；§4.5 说的「复用」只复用 `Content-Range` 里的 `length`，字节没有复用）。
  最坏延迟 3 × 4 s = 12 s/条目、7 条目 ≈ 84 s，且**在下载开始前串行发生**。
  尺寸最不成比例的是 `moss/cpu`：只有 1,121,692 B，却要先下 6 MB 去决定怎么下它。
- **修复建议**：小条目跳过测速（直接走 `Test-RangeSupport` 的旧路径）。
- **修复后代码示例**：

  ```powershell
    # 条目比探针还小 ⇒ 测速毫无意义（探针量是目标量的好几倍），直接探 Range 能力
    $probeMB = $ProbeMB
    if ($expectedSize -gt 0 -and $expectedSize -lt (8 * $ProbeMB * 1MB)) { $probeMB = 0 }
  ```

### 🟢 P2-2 探针报的总长无校验地覆盖清单 `size`，异常 CDN 可让段数爆炸

- **位置**：`scripts/bootstrap_runtime.ps1:686` 对照 `:698-700`、`:714`
- **问题**：`if ($ranked[0].Length -gt 0) { $expectedSize = $ranked[0].Length }` —— 清单里已有可信
  `size`（且启动时的磁盘预检用的就是它），这里却让探针的 `Content-Range` 直接顶掉。
  一个回 `Content-Range: bytes 0-2097151/999999999999999` 的 CDN 会让 `:699` 算出天文数字的段数，
  `:714` 的循环逐段建文件直到磁盘满，而预检（按清单 size）不会拦。
- **修复建议**：与清单 `size` 交叉校验，不一致以清单为准并记一条日志。
- **修复后代码示例**：

  ```powershell
      if ($ranked[0].Length -gt 0) {
        if ($it.size -and $ranked[0].Length -ne [int64]$it.size) {
          Log "探针总长 $($ranked[0].Length) 与清单 $($it.size) 不一致，以清单为准"
        } else {
          $expectedSize = $ranked[0].Length
        }
      }
  ```

### 🟢 P2-3 `$SegmentMB = 0` 是函数局部赋值，日志会重复 7 次，且「整个运行切单流」的意图未生效

- **位置**：`scripts/bootstrap_runtime.ps1:629-632`
- **问题**：PS 函数内给参数/脚本域变量赋值只影响本次调用。实测：脚本域 `$SegmentMB = 64`，
  函数内改成 0 返回后仍是 64，于是 7 个条目每次都会重新进这个分支、重复打印 7 遍
  「未找到 curl.exe…」。行为本身不算错（本次调用内确实是 0），但意图与实现不符，
  而且这条噪音会掩盖真正的失败信息。
- **修复建议**：写脚本域变量。
- **修复后代码示例**：

  ```powershell
    if ($SegmentMB -gt 0 -and -not (Get-Command curl.exe -ErrorAction SilentlyContinue)) {
      Log "未找到 curl.exe（Win10 1803+ 自带），本次运行退回单流下载（无分段、无续传）"
      $script:SegmentMB = 0
    }
  ```

### 🟢 P2-4 `:454` 的字段数守卫是死的；真正拦住失败的是 `:459`

- **位置**：`scripts/bootstrap_runtime.ps1:454`
- **问题**：`if ($parts.Count -lt 4)` 看上去像在处理网络失败，但实测 curl 在连接被拒（exit 7）、
  DNS 失败（exit 6）、超时（exit 28）时**照样输出 `-w` 行**（`000 0 0.000553 0.000550` 等），
  4 个字段齐备，守卫永不触发。真正判定失败的是 `:459` 的 `$code -ne 206`（`[int]"000"` = 0）。
  行为正确，但注释不点明的话，下一个人会以为超时已经被处理了。
- **修复建议**：把注释写实（`000` 是 curl 的失败状态占位，由后面的 206 判定拦住），
  或直接删掉这个守卫。

### 🟢 P2-5 新增的 `-ProbeMB` / `-ProbeSec`（以及文档承诺的 `-SegmentMB 0`）用户摸不到

- **位置**：`scripts/bootstrap_runtime.ps1:64,67,68` 对照 `src-tauri/src/ai_runtime/runtime_install.rs:711-726`
- **问题**：Tauri 后端只传 `-Items -Flavor -RuntimeDir [-Manifest] [-Force]`，
  `src-tauri/` 下 `git grep` 找不到 `-ProbeMB` / `-ProbeSec` / `-SegmentMB` 任何一处。
  于是：慢链路用户无法把探针调小以缩短 84 s 探测；运维无法在 gh-proxy 被打死时切源；
  契约里写明的 `-SegmentMB 0`（关闭分段）也没法从界面触发。
  **这一点同时限定了 P0-1 的爆炸半径**：`-SegmentMB 0` 只能由「curl 缺失」间接触发，
  而不是由设置面板触发。（设置面板下发 `-SegmentMB` 是 PR #52 做的，两个 PR 合并后才补齐。）
- **修复建议**：`RuntimeConfig` 增 `probe_mb`/`probe_sec` 并由 `runtime_install.rs` 显式下发
  （与该处既有注释「显式传参而不是靠脚本默认值」一致），或在契约里明确「这两个参数仅供命令行/运维使用」。

### 🟢 P2-6 清单元数据过期，且 `source_version` 会把 11-14 的内容记成 11-02 的版本

- **位置**：`src-tauri/resources/runtime/manifest.json:2-3`（`"version": "1"`、`"generated_at": "2026-11-02T01:37:25Z"`）
- **问题**：7 条 `urls[]` 全部在 2026-11-14 重写，但这两个字段与基线 `9cae08c` 逐字节相同。
  **对正确性无影响**（Rust 侧不读这两个键，只取 `id/flavor/sha256/size/unpacked_size`；
  安装器只把 version 打进日志 `scripts/bootstrap_runtime.ps1:161-162`，跳装判据是 `sha256`），
  但 `$manifestVersion` 会被写进用户状态作为 `source_version`（`:840`，
  类型见 `src-tauri/src/ai_runtime/installed_state.rs:40`）——所有从这份清单安装的机器
  永远记着 `source_version: "1"`，与 11-02 的安装无法区分。`generated_at` 还会被
  `scripts/package_moss_flavors.ps1:233-237` 带进 `packages.json`，下游出处信息因此声称
  11-14 的产物是 11-02 的。
- **修复建议**：把 `generated_at` 更新到真实上传日，`version` 改成日期戳；
  或在契约 `:117` 注明这两个字段仅作参考。

### 🟢 P2-7 仓库里没有任何东西能重新生成这份清单，重跑打包脚本会静默删掉本 PR 的全部成果

- **位置**：`scripts/package_moss_flavors.ps1:186-195`（只产 `id/flavor/url/sha256/size/dest/backend_marker`）、`:222-228`（写出时 `version="1"` + 新 `generated_at`）
- **问题**：该打包脚本不产 `urls`、不产 `unpacked_size`、不产 `optional`，也不产
  `models-moss`/`deps-ocr`/`deps-ocr-gpu`/`deps-embed` 四个条目；`scripts/` 下没有任何脚本写
  `src-tauri/resources/runtime/manifest.json`；`.github/` 里零清单引用，也没有 CI 检查。
  于是这张 7×3 的 URL 表成了**没有生成器、没有守卫的手工生产数据**——
  下一个人重跑打包脚本并覆盖清单，全量首装对所有用户即刻失效，且没有任何测试会红。
- **修复建议**：让打包脚本产出 `urls[]`（或加一个 `scripts/update_manifest_urls.ps1`），
  并加一条校验：每条资产必须有 3 个 `urls` 且 host 顺序符合预期。

### 🟢 P2-8 设计文档 §8.13 的两条旧结论被本 PR 推翻但未标注

- **位置**：`docs/engines_vulkan_design.md:468`（与 `_EN.md:486`）、`docs/engines_vulkan_design.md:397-406`（与 `_EN.md:398-408`）
- **问题**：
  1. `:468` 「⇒ 所以 `urls[]` 顺序是 **ESA → GitHub**」——已被 §8.14 的
     「ModelScope → ESA → GitHub」取代，两处相隔 20 行、结论相反，且没有取代标记。
     对照 `docs/MISC_TODO.md:630` 是**加了**「2026-11-14 追加」标记的（所以本 PR 知道要做这件事，
     漏在了设计文档这一侧）。
  2. `:397-406` 「每段先 `curl -r 0-0` 探 206」——本 PR 之后是**每候选探一次**、
     且总长直接复用（契约 `:328` 就是这么写的）。这句话出现在 `:397-406`，
     同时是选源规则的第三份副本（另两份在契约 `:172`、`:267`），三份互不一致。
- **修复建议**：`:468` 追加取代标记；`:397-406` 改成「每候选探一次，总长复用」。

### 🟢 P2-9 契约里的清单示例一个 `urls` 都没有

- **位置**：`docs/RUNTIME_BOOTSTRAP_CONTRACT.md:114-164`
- **问题**：示例 JSON 含 `version`/`generated_at`/`id`/`flavor`/`optional`/`url`/`sha256`/
  `size`/`unpacked_size`/`dest`，**不含 `urls`**（`urls` 只在 `:172` 的字段表里出现）。
  本 PR 之后 `urls[]` 是生产下载的承重结构（三级 + ModelScope 主源），
  但照着这个示例抄出来的清单是单源的，会静默失去本 PR 的全部收益。
- **修复建议**：在示例里补一个三元素的 `urls` 数组，并用注释标出 `ModelScope → ESA → GitHub`。

### 🟢 P2-10 两张 ESA 实测表的速率口径不同，读者会误判「ESA 变慢了」

- **位置**：`docs/engines_vulkan_design.md:537-546` 对照 `docs/RUNTIME_BOOTSTRAP_CONTRACT.md:304-311`
- **问题**：设计文档新增的逐条表标注「`installed_at` 差值，**含解压**」，
  但每行速率都比 `大小 ÷ 耗时` 高 5–6 %（moss 621.6 MiB / 7.5 min = 1.38 vs 标 1.46；
  deps-embed 0.37 vs 0.39），即**速率列只按纯下载时长算、时间列含解压**——同一行两个口径。
  而契约 `:304-311` 那张老表内部自洽（621.6/61 = 10.19 ≈ 10.2）。两张表口径不同却都没有说明。
  顺带：`:549-550` 的「两轮落盘同为 6.34 GB」没说是十进制 GB 还是 GiB、也没说是否计入
  两个 moss 条目展开后的体积（它们根本没有 `unpacked_size`），无法复算。
  另有两处同源不同结论的数字未互相解释：契约 `:313` 的「全量 6 项 19.3 分钟」与
  设计 `:534` 的「ESA 41 分 50 秒」差 2.2 倍（不同日期、后者含解压）。
- **修复建议**：给速率列标注口径「（按纯下载时长计）」，并给 6.34 GB 补分母说明。

### 🟢 P2-11 `verify_cdn_mirror.ps1` 正好是本 PR 叙事里「验证一个源」的工具，却零引用，且缺文档要求的那段

- **位置**：`scripts/verify_cdn_mirror.ps1`（唯一引用是自身头部的示例 `:15-16`）
- **问题**：它会探 Range、报 TTFB 与持续速率、读 `Content-Range` 总长（`:79-82`）、算 ETA（`:120-122`），
  正是本 PR 在契约 `:347`、`:350` 与设计 `:524-525` 反复要求的「手工验证候选源」。
  但两处文档都没提它。另外它**没有**实现设计 `:524-525` 要求的那句
  「重试到 `Content-Range` 总长等于本地字节数为止」（ModelScope 刚上传会 404 传播延迟）。
- **修复建议**：在契约 `:350` 与设计 `:524` 引用该脚本；把重试循环补上或在文档里注明缺口。

## 已核对无误的点（抽查清单）

| 检查项 | 结论 |
|---|---|
| **7 条资产的 `urls[]` 全部 3 条、顺序一律 ModelScope → dl.rpegio.top → GitHub**（键在 `manifest.json:13,27,41,53,67,82,95`） | ✓ 无一条漏改或错序 |
| **ModelScope 直链真的能下同一份字节**：7 条 `urls[0]` 均 302 → `cdn-lfs-cn-1.modelscope.cn`，`Range` 请求返回 **206**，`Content-Range` 总长与 `manifest.size` **逐条精确相等**（1121692 / 16139894 / 651834262 / 626397892 / 309803561 / 502533301 / 157386205） | ✓ 独立 curl 复核 |
| **LFS 对象路径 `/xx/xx/<60 hex>` 里的 sha256 与 `manifest.sha256` 逐条吻合** | ✓ 7/7 全对，换源没有换字节 |
| `sha256` 校验与选源无关、每次下载后都跑：`scripts/bootstrap_runtime.ps1:808-815` `HASH_MISMATCH 期望 … 实际 …` | ✓ 完整性没被削弱 |
| 单数 `url` 仍指 GitHub（供 `-Mirror` 前缀与直连兜底），7 条全部一致 | ✓ |
| `size`/`unpacked_size`/`sha256`/`dest`/`backend_marker`/`flavor`/`optional` 与基线逐字节相同（本次 diff 只往 `urls[]` 前插一行） | ✓ |
| `$mirror` 遮蔽 `-Mirror` 参数的 bug **在本分支不存在**：`Get-UrlCandidates`（`:397-412`）直接用参数 `$Mirror`，无同名局部赋值 | ✓ 那是 PR #52 的问题，见「合并风险」 |
| `Test-RangeSupport` 的 `Content-Range` 解析（`:432`，`0-0` 不会被裁短）精确，`-ProbeMB 0` 路径拿到的 `Length` 可信 | ✓ |
| 分段失败后的重排**一个候选都不丢**：`:757` 把失败源排到队尾，实测 4 条全在 | ✓ 这是本 PR 改得对的地方 |
| `:684` 的 `$ranked[0].Kind -ne "mirror"` 成功阻止了用镜像做分段（镜像 Range 只有 0.18 MB/s） | ✓ 判断本身正确 |
| `finally` 清理探针临时头文件（`:471-473`）覆盖所有出口：正常、`--max-time` 中止、异常、`-ProbeMB 0` 早退；实测 TEMP 无泄漏 | ✓ |
| `[int64]` 溢出不会崩：20 位数字的 `Content-Range` 组抛转换异常，被 `:461-464` 内层 `catch` 吞掉，`$len` 保 0 | ✓ |
| 除零/负分母已守：`:465-467` 覆盖 `ttfb > total` 与 `xfer -le 0` | ✓ |
| 200 整包、404/500、连接被拒（exit 7）、DNS 失败（exit 6）、超时（exit 28）全被判 `ok=false` | ✓ 实测逐个通过 |
| `@()` 防展开到位：`:636` `@(Get-UrlCandidates $it)`、`:399` `foreach ($u in @($it.urls))`，单元素/空值不会塌缩 | ✓ |
| 不等速排序正确（9.0 / 5.0 / 1.5 → ESA → GitHub → ModelScope） | ✓ 只有等键时才乱（见 P1-1） |
| 探针跳过条件与文档一致：`curl` 缺失（`:629-632`）、`-SegmentMB 0`（`:644`）、套镜像（`:648-650`） | ✓ 且 `-SegmentMB 0` 的跳过嵌在 `-ProbeMB` 分支之上，优先生效 |
| 契约 `:330` 的「失败回退按测速排序的剩余顺序」与 `:750-759` 一致 | ✓ |
| 文档所有数字可复算：**5.08×**（2510 s / 494 s）、**2.09 GiB**（5 个 zip 之和 = 2,247,955,221 B）、逐条 MiB 大小、段数 10/10/5/8/3、逐条耗时合计 41.9 min vs 41 m 50 s | ✓ 全部对得上 |
| 中英设计文档 §8.13/§8.14 逐节对等（表格、数值、6 条上传要点、结论三点无删减）；EN 的 +134 比 CN 的 +81 大是因为**基线 EN 根本没有 §8.13**（`9cae08c:docs/engines_vulkan_design_EN.md` 到 §8.12 就结束），属正常补译 | ✓ |
| `docs/` 下没有失效的 `file:line` 引用（`git grep "bootstrap_runtime\.ps1:[0-9]"` 空结果），+101 行的位移没打断任何引用 | ✓ |

## 验证（本审查独立执行）

- **P0-1 / P1-1 / P1-2 用真实 `powershell.exe` 5.1.26100.9818 复现**：从 `2a8306e` 逐字抽出
  `Get-UrlCandidates`(`:397-412`) + `Invoke-SegmentedDownload` 的选源段(`:643-681`)，
  喂真实 `src-tauri/resources/runtime/manifest.json` 条目，跑 6 组输入（默认 / `-SegmentMB 0` /
  `-SegmentMB 0 + -Mirror ""` / 探针全失败 / 部分失败 / `-ProbeMB 0`），得到上文的
  1 / 3 / 4 条候选对比。
- **`Sort-Object` 稳定性**独立实测：等键 n = 3/4/5/8 全部不保序（n=3 直接整体倒序），
  不等键正确；同一输入重复三次结果一致；**`-Stable` 参数在 PS 5.1 上不可用**。
- **探针边界**用真实 curl + 真实互联网跑：正常源 `206 1121692 0.829610 0.674615`（4 字段齐、
  `LASTEXITCODE=0`）、连接被拒 `000 0 …`（exit 7）、黑洞 `000 0 2.012203 …`（exit 28，
  全部被 `:459` 正确拒绝，无未捕获异常）；`--max-time 0` 实测不限时（会在探针上无限期挂住）；
  `--max-time -5` 被 curl 拒绝（`expected a proper numerical parameter`）。
- **清单数据独立复核**：7 条 `urls[0]` 逐个发 `Range` 请求，比对 206 + `Content-Range` 总长 +
  LFS 路径内编码的 sha256（结果见上表）。
- **试合并**：临时 worktree 里 `git merge --no-commit --no-ff feat/runtime-distribution`
  → 零冲突，但合并结果的 `Get-UrlCandidates` 是 PR #52 那版（含 `$mirror` 遮蔽）。
- **文档漂移**逐条按「文件:行号」点开核对，并用 `git show 9cae08c:` 对照基线。
- 未做真机 GUI 验证（本环境无真机）；下载器行为结论来自逐字抽代码实跑 + 提交里的实测记录采信。

## 结论

**P0-1 已修复（2026-11-16），当前建议：修完 P1 后再合并。**
原始 P0-1 的处置是把兜底判据从 `$ranked.Count` 改成 `$probed.Count`、并把镜像追加挪到兜底之后
（`scripts/bootstrap_runtime.ps1:673-703`），修复后 `-SegmentMB 0` 与「探针全失败」两条路径
的兜底队列从「mirror 1 条」恢复成「ModelScope → ESA → GitHub → mirror 4 条」，
端到端实跑也已选中 ModelScope。它原本让「关闭分段」和「无 curl」这两条契约写明的正式路径
**只依赖第三方代理**，而本 PR 恰好把一条真实直链设成了主源。

P1-1 ~ P1-3 是同一个函数里三处「测量不可信」（排序不稳定、失败即删除、假高速），
建议同批修：它们决定「选哪条源」，而这个 PR 的全部价值都压在这一次选择上。
P1-4 ~ P1-6 是文档：契约 §4.5 是唯一权威描述却与实现四栏不符，字段表的关键语义被本 PR 推翻，
设计文档留着一条彻底的假话——**建议随本 PR 一起修**，因为这三处正是后来者的前提。

P2 共 11 条，绝大多数可以合并后按档案规矩逐条跟进。其中 **P2-7（清单没有生成器也没有 CI 守卫）**
和 **P2-6（`source_version` 会把 11-14 记成 11-02）** 建议优先，因为它们让这份手工维护的
三级 URL 表在下一次「重新打包清单」时静默失效，而没有任何东西会报警。

**合并顺序建议**：先修 PR #52 的 P0-1（`$mirror` 遮蔽），再合本 PR。否则本 PR 里
「镜像只做单流兜底」这条设计会被 PR #52 的 bug 架空，两边的 P0 交织成一个更难查的症状。

## 处置记录（待用户裁决后回填）

> 按 `AGENTS.md` 的档案同步规矩：**修复提交与本文档的状态行必须同一提交更新**，
> 状态行要写依据（提交号 / PR 号），不要只写「已修复」。

- **P0-1** ✅ **已修复**（2026-11-16，PR #53 内，`scripts/bootstrap_runtime.ps1:673-703`）——
  兜底判据从 `$ranked.Count` 改为 `$probed.Count`：`$probed` 为空时先用**全部非镜像直链**打底
  （`:681-688`），镜像改为在兜底**之后**才追加（`:689-694`），分段源额外加 `$probed.Count -gt 0`
  守卫（`:699`）。修复后复跑同一套用例矩阵（真实 `powershell.exe` 5.1，
  逐字抽取 `:673-704`）：`-SegmentMB 0` 与「探针全失败」从 **mirror 1 条**变成
  **ModelScope → ESA → GitHub → mirror 4 条**；`-SegmentMB 0` 端到端实跑选中 ModelScope
  （修复前必然是 gh-proxy）。处置提交号待用户手动提交后回填
- **P1-1** ✅ **已修复**（2026-11-16，PR #53 内，`scripts/bootstrap_runtime.ps1:668-695`）——
  `-ProbeMB 0` 不再进速率排序（直接沿用插入序），需要按速率排序时显式构造 `Seq` 次键：
  `Sort-Object @{ Expression = "Mbps"; Descending = $true }, @{ Expression = "Seq"; Descending = $false }`
  （面板里不用 `-Stable`，PS 5.1 没有它）。日志与下载队列共用同一次排序结果，避免两处顺序不一致。
  复测（真实 `powershell.exe` 5.1，逐字抽取 `:668-695`）：等键 n=3/5 由「整体倒序 / 乱序」变为
  **严格保序**（ModelScope → ESA → GitHub、A→B→C→D→E），夜间 GitHub 与 gh-proxy 等速的用例
  得到 ModelScope → ESA → GitHub，`-ProbeMB 0` 完全保序，不等键（9.0/5.0/1.5）仍正确降序
- **P1-2** ✅ **已修复**（2026-11-16，PR #53 内，`scripts/bootstrap_runtime.ps1:446-475` + `:659-671`）——
  `Measure-UrlSpeed` 的每个返回点都带上 `rejected`：拿到 HTTP 状态码（200/4xx/5xx）才为 `true`，
  `code = 0` 的连接失败 / 超时 / 零字节 / 计时不可信为 `false`（`:462`）。调用侧据此分三支
  （`:659-671`）：`ok` 正常入队；`-not rejected` 记 `Slow` 保留在队尾并打
  「未测到速率…（保留为单流兜底）」；只有 `rejected` 才真丢弃。`Slow` 一路带到 `$ranked`
  （`:702`），分段选择额外要求 `-not $ranked[0].Slow`（`:722`），防止慢源被选去分段。
  复测（真实 `powershell.exe` 5.1，逐字抽取 `:643-723`，用桩函数喂三种探针结果）：
  超时源从「消失」变为**保留在队尾**（`ESA → GitHub → ModelScope`，ModelScope 标 `Slow`）；
  404 源仍被丢弃；**全部未测到时三名直链全部保留且不选源分段**（`rangeUrl = $null`）；
  真实网络端到端实装 `moss/cpu`：测速 5.14 / 0.43 / 0.28 MB/s，选 ModelScope，`EXIT=0`
- **P1-3** ✅ **已修复**（2026-11-16，PR #53 内，`scripts/bootstrap_runtime.ps1:463-475`）——
  ① 正则放宽成 `bytes\s+0-\d+/(\d+)`（`:467`），服务端裁短或对象小于探针的合法 206 也能取到总长；
  ② 取不到 `Content-Range` 直接判「没测出来」（`:469-472`）；
  ③ 新增可信度门槛：`$want = min($last+1, $len)`，`$size < $want * 0.5` 即判「没测出来」（`:474-475`），
  不再让「回得少、回得快」算出假高速。两类都只降级为 `Slow`，不判 `rejected`（与 P1-2 同一语义）。
  复测（真实 `powershell.exe` 5.1，逐字抽取 `:446-482`，用桩 `curl.exe` 喂构造响应）：
  正常 206 与「对象小于探针的整包 206」仍 `ok=True` 且总长正确；裁短到 1 KiB 的 206
  **由 0.705 MB/s `ok=True` 变为 `ok=False`（`Slow`）**；无 `Content-Range` 的 206 由
  897 MB/s `ok=True` 变为 `ok=False`；200/404 仍 `rejected=True`，超时仍 `rejected=False`。
  ⚠ 契约 §4.5 的探针行在本提交内未改（同一行的四栏订正由 P1-4 一次性重写，见下条）
- **P1-4** ⚠ 未修复（`docs/RUNTIME_BOOTSTRAP_CONTRACT.md:326` 探针行四栏与实现不符；`-ProbeSec` 无文档）
- **P1-5** ⚠ 未修复（`docs/RUNTIME_BOOTSTRAP_CONTRACT.md:172` 「顺序即优先级」被 `:349` 与代码推翻）
- **P1-6** ⚠ 未修复（`docs/engines_vulkan_design.md:406` 仍写 ModelScope「已调研未落地」）
- **P2-1 ~ P2-11** ⚠ 未修复（见上；建议 P2-6 / P2-7 优先）
