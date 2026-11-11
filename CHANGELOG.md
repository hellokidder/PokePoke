# Changelog

> 中文 | [English](./CHANGELOG.en.md)

本文件记录 Poke Poke 的**用户可见变更**，会随 Release 对外发布（应用内更新提示同样展示此内容），格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)。

## 写入原则

**记什么**：只记用户能感知的行为变化（其余改动靠 git log 即可）：

- 新增 / 移除终端、agent 或功能支持
- Popup 触发 / 关闭 / 抑制行为变化
- 设置项新增、删除、默认值调整
- 用户可见的 bug 修复

**怎么写**：从用户视角描述「现在可以做什么 / 修好了什么」，一行一条，简洁为主。

**不写**：

- 实现机制与内部细节：hook 事件、环境变量、配置文件路径、CLI 命令、优先级矩阵、schema 字段名
- 开发 / 运维向内容：诊断导出、内部 command、测试与重构
- brief / PR / commit 引用（这些在 git 历史与 `docs/briefs/` 里查）

## 类别

- `Added`：新增行为
- `Changed`：已有行为的变更
- `Deprecated`：即将废弃
- `Removed`：已移除
- `Fixed`：修复（带行为修正）
- `Security`：安全相关

---

## [1.1.6] - 2026-11-11

### Added

- 支持 **DeepSeek Harness(DSH)**:会话状态提醒、审批与提问提醒,点击提醒直接切回对应对话
- **开源**:源码以 MIT 授权公开,可自由使用、修改、再分发

### Changed

- **全部功能免费**:取消每日提醒次数上限与 3 天试用,不再需要 License 才能无限提醒
- DSH 的「完成」判定更准:子会话或后续轮次还在跑时,不再提前弹完成提醒

### Removed

- 设置里的「License」页、面板上的额度提示、以及「升级 Pro」入口全部下线;
  升级后这些界面自动消失,无需手动处理

---

## [1.1.5] - 2026-10-31

### Added

- 支持 **oh-my-pi(omp)** agent:会话状态提醒、完成弹窗、审批请求提醒,点击跳回对应终端

### Changed

- Ghostty 回跳更准:同一个目录开着多个 Ghostty 标签页或分屏时,点提醒也能回到发出提醒的那个标签页/分屏(Ghostty 1.4 及以上)
- 提醒出现更快:agent 完成一轮后不再等待提醒自身的窗口定位 / 聚焦探测,
  从完成到提醒弹出的可感知延迟缩短约 200ms(提醒的触发与抑制行为不变)
- 提示音预热更短:设备空闲后第一声的前置延迟从约 300ms 降到约 100ms,减少声音到来前的空等

### Removed

- 设置里的「Ghostty 回跳增强」已下线:它靠改写终端标题提升回跳精度,现在不再需要;升级后该开关自动消失,无需手动处理

### Fixed

- 修复 Claude Code 权限确认不及时提醒的问题:确认框一出现就弹提醒(此前要等约 6 秒;
  6 秒内自行确认时完全不弹)
- 修复 Cursor CLI 会话的提醒「闪一下就没」、会话面板里也没有记录的问题
  (IDE 未打开时,该会话约 10 秒后会被误判成已结束)
- 修复在 cmux 中点击会话(popup 或列表)无法跳回对应 pane 的问题,尤其是应用从 Dock 启动之后
- 修复 Orca 未运行时每次提醒都白等一次探测的问题(每次提醒多花约 165ms)
- 修复 pi 中手动取消对话(Ctrl+C)后仍播放「任务完成」音效的问题
- 修复提示音偶发「开头被吞、像从中间开始响」的问题(设备空闲后的第一声)
- 修复聚焦终端时一次权限请求偶发响两次提示音的问题(agent 重发待处理通知导致)
- 修复会话面板里的免费提醒计数不跟随使用更新的问题(此前要重启才看得到新值,现在实时刷新)

## [1.1.4] - 2026-08-19

### Added

- 支持 **Warp** 终端:点击会话即可跳回对应窗口
- 支持 **tmux** 会话:状态提醒与点击跳回
- 支持 **herdr 远程会话**:通过 SSH 转发连接远程机的 herdr,在本地弹完整
  popup(远程机零安装)

### Fixed

- 修复任务完成后提示音偶发被吞的问题:agent 在等待操作提醒后很快完成时,
  完成音效现在仍会正常播放(之前可能只听到等待音)
- 修复 Cursor 任务被 Ctrl+C 取消后误播完成音 / 重复播放提示音的问题
- 修复 herdr 一次对话连弹三次「完成」popup 的竞态问题
- 修复 herdr 会话承载在 Warp / cmux 中时点击跳回无反应或停错 tab 的问题
- 修复 herdr 远程会话在部分情况下无法被发现、收不到提醒的问题
- 修复重启后 herdr 会话回溯弹「干完了」popup 的问题

## [1.1.3] - 2026-08-03

### Added

- 支持 **Otty**、**Orca** 终端:点击会话即可跳回对应窗口,已聚焦对应终端时不重复弹窗
- 支持 **Kimi Code**、**Grok Build**
- 支持自定义提示音:可导入本机音频作为提示音

### Changed

- 提示音升级:内置多套音效,并可为「进行中 / 空闲 / 失败」三种状态分别设置提示音

### Fixed

- 修复 Kimi 会话弹窗误判、以及多个会话同时运行时状态记录错乱的问题
- 修复自定义提示音偶发丢失的问题

## [1.1.2] - 2026-07-27

### Added

- 支持 **pi** agent:会话状态提醒、完成弹窗、点击跳回对应终端
- 支持 **WezTerm** 终端:点击会话即可跳回对应窗口,已聚焦对应终端时不重复弹窗

## [1.1.1] - 2026-07-22

### Added

- **静谧模式**:新增倒计时静音功能;倒计时内提示窗照常弹出但不播放提示音;全局快捷键默认 `⌘;`,可在设置"通用"页独立配置(与「提示热键」前缀解耦)。
- **接入端手动断开生命周期**:tray 菜单与设置面板均可对 CC / Codex / Cursor / OpenCode 显式断开,断开后 `/notify` 直接拒收且不会被首启补装。
- **快捷键最小修饰键校验**:popup 前缀 & 静谧模式热键均拒绝纯 Shift/Alt 组合(必须含 ⌘ 或 Control),避免普通输入误触。

### Changed

- **hook 首启安装带版本校验**:内嵌版本 ≤ 已装 hook 时跳过覆盖,防止 dev 环境把 release 装的 hook 反向降级。
- **popup 聚焦抑制扩展**:cmux / herdr / Cursor 也进入抑制矩阵;herdr pane 抑制要求宿主 app 处于前台。

### Fixed

- **首启无 agent 时不再触发假装安装**:未检测到任一 agent 时跳过 first-run install 与 auto-connect。

## [1.1.0] - 2026-07-16

### Added

- **herdr / cmux pane 内的 CC 回跳**:CC 跑在 herdr / cmux 复用器 pane 时可精准跳转,直接在 Ghostty/iTerm2/Terminal 跑的会话不受影响。
- **Codex 桌面端精准回跳**:点击提醒后通过 `codex://threads/<id>` 精准回到对应对话。

### Changed

- **Codex 自动审批轮次抑制 popup**:进入自动审批的轮次不再弹审批提示,判据拿不到时降级放行。
- **Ghostty reconcile 指数退避**:周期对账连续失败时改指数退避,避免高频空转。
- **升级重启后 hook 自动刷新**:收敛旧版 debug hook 残留覆盖。

### Fixed

- **popup 竞态保护**:同 session popup 已在屏时,refresh-only 不重建窗口、不重复播音。
- **`focus_check` AppleScript 语法**:修复部分系统版本上的语法解析失败。

## [1.0.2] - 2026-07-08

### Changed

- **多 tab 同项目时点击提醒跳错窗口**:同一项目开了多个终端 tab,冷启动首轮若抓错窗口标识,之后点击提醒会一直跳到错误的窗口,直到那个窗口被关掉。现在改用 tab 标题作为强证据自动纠正错绑。
- **关闭的 agent 会话残留面板、提醒关不掉**:agent 退出后,若它的会话没带终端信息,只要机器上还有同名的其它进程就会被误判"还活着",导致面板残留死会话、提醒弹窗关不掉。现在无终端信息的会话直接判为已退出,下次有新事件会重新出现。
- **切换终端后提醒被误关**:之前在 Ghostty 用过、后来换到 iTerm2/Terminal 的会话,若前台恰好是旧的 Ghostty 窗口,会被误判"用户正在看"而抑制或关掉提醒。现在按当前实际终端类型判断焦点,不再被旧标识干扰。
- **agent 报错失败被静默吞掉**:极少数情况下,第三方包装在 100ms 内先报"一轮结束"再报"API 出错",失败状态会被去重当成重复丢弃,用户看不到失败提醒。现在去重把"正常结束"和"出错失败"区分开,失败提醒不再被吞。
- **第二实例启动会干扰主实例**:此前再开一个 PokePoke 时,新实例在退出前会去清会话、改接入配置,把主实例的状态搅乱。现在第二实例启动即立刻退出,不碰任何共享状态。
- **老用户升级后被误判首次启动、自动重连已断开的接入端**:从未在设置里改过任何项的用户升级后,因设置文件缺失被当成首次启动,自动重装全部接入(可能重新连上你手动断开过的工具)。现在只要用过一次(会话记录已落盘)就不再误判。
- **启动时刚出现的会话被误清**:启动清理与接收事件的时序收窄窗,避免刚报上来的真实会话被当历史残留删掉。
- **卸载接入时误删用户自己的 hook 条目**:此前按"组"删除,若你手动把自己的 hook 条目放进 PokePoke 的同组会被一并删掉。现在只删 PokePoke 自己的条目,保留你的。
- **接入配置过期检测更严**:此前只要命令里带来源参数就算正常,旧路径的二进制也被误报健康。现在要求命令字符串完全一致,路径或参数任何偏差都会提示一键修复。

### Added
- **新增 opencode 接入端**:在托盘菜单可一键 Connect / Disconnect / Check opencode。opencode 的会话生命周期事件(开始工作、等待权限审批、一轮结束、出错)会像 Claude Code / Codex / Cursor 一样聚合到面板并弹提醒。opencode 是除 Claude Code 外唯一能在出错时触发失败提醒的接入端。首次启动自动接入,配置过期会提示修复。

## [1.0.1] - 2026-07-05

### Fixed
- **focus-watcher 关联 `focus_suppression_enabled`**:关掉聚焦抑制时,watcher 不再后台秒关 popup(此前 popup"闪一下"消失)。
- **Stop 带未完成 `background_tasks` 时不弹"完成"**:子 agent / 后台 shell 仍在跑时,Stop 只是 turn 边界,保持 session Running。
- **CC/Cursor/Codex install 对损坏配置防 panic**:`settings.json` / `hooks.json` / `config.toml` 非 object / 非 table 时降级到空对象,不再崩溃。
- **Creem / `codex login status` HTTP 加超时**:不再因 API 不可达无限挂起(后者原首启主线程同步跑,挂住 = 窗口永不出现)。
- **Settings 写失败回滚**:`saveField` 后端写失败时回滚 UI 状态,不再显示新值但实际没落盘。

### Security
- **`settings.json` / `sessions.json` 收紧 0600**:此前 umask 残留 0644,飞书凭证 / workspace 路径可被同机进程读取。对齐 `license.json` 既有 0600 模式。


- **Settings 窗不再切 activation policy**:全程 Accessory,消除 popup 弹出顺带抬高 settings 的副作用;不进 Dock/Cmd+Tab(入口在 tray)。窗尺寸 560×600 → 620×720。

## [1.0.0] - 2026-07-01

首个正式版本。商业化收款正式切到 Creem 生产环境。

### Added

- **首启按系统语言自动选中英**:全新安装时 `default_locale()` 用 `sys-locale` 探测系统语言(`zh*` → 中文,其余 → 英文,取不到兜底中文);前端首帧初值改为 localStorage 优先、否则按 `navigator.language` 探测,消除英文系统下的首帧中文闪烁。仅影响全新安装/全新访客,已有 `settings.json` / localStorage 行为不变。



- **License 收款切到 Creem 生产环境**:`POKEPOKE_CREEM_API_KEY` 用正式 `creem_xxx` key(无 `_test_` 前缀 → 走 `api.creem.io` 正式端点);product_id 映射改用正式商品。
- **购买入口改为月付/买断双链接**:Settings「升级 Pro」由单按钮改为「月付 / 买断」两个按钮,`open_purchase_url` 接 `kind`(monthly/lifetime)并由 `POKEPOKE_CREEM_*_PRODUCT_ID` 派生 `https://www.creem.io/payment/{product_id}`;landing page 两个 checkout 链接同步为正式商品。

### Removed

- **移除 `POKEPOKE_CREEM_PURCHASE_URL` 环境变量**:购买链接改由 product_id 派生,不再需要单独配置该项。

### Fixed

- **Codex 安装不再覆盖用户已有 hooks**:`--install-codex`(含首启自动接入)此前用全新配置整体覆写 `~/.codex/hooks.json`,会静默抹掉用户在 Codex 里手写的所有 hook;现改为读取现有配置后按事件合并(移除旧 PokePoke 条目再追加,幂等),保留用户在其它事件/同事件里的自定义 hook,与 Claude Code / Cursor 的合并语义一致。
- **Ghostty 多窗口回跳误改另一窗口的选中 tab**:焦点在窗口 B 时回跳窗口 A 的对话,会把 B 的 selected tab 也换掉。根因:`focus_ghostty` 的 L1/L2 脚本在 `repeat with aWindow in windows` 遍历时把 `targetTab`/`targetTerminal` 一起存下,这些是按当前 z-order 索引的延迟解析引用;随后 `activate window targetWindow` 把目标窗口拉到前台、windows 顺序翻转,之后 `select tab targetTab` 复用旧引用会按新 z-order 重新解析,错位命中被覆盖窗口的同索引 tab(实测可复现:目标在 A.tab1、焦点在 B 时,B 的 selected tab 被换成 tab1;若 B 的 tab 数不足则报"无效的索引")。修复:遍历只存 `targetWindow`,`activate window` 翻转 z-order 后在 `tabs of front window` 里按 id/cwd **重新解析** tab/terminal 再 `select tab`+`focus`,绝不跨翻转复用引用。

## [0.0.9] - 2026-06-30

### Added

- **首启 3 天 Pro 试用期**:首次启动自动给 3 天 Pro 试用,过期后回落到免费层每日通知配额。
- Claude Code transcript 出现未恢复中断标记时，以中性 `pending` 提醒用户返回查看/继续。transcript watcher 只扫描 `running` 的 Claude Code session，检测尾部 `[Request interrupted by user]` marker，复用 `/notify` 链路触发 popup/sound。
- **Ghostty terminal_id 周期对账**(brief `20260629-ghostty-jump-reconcile`):新增后台 watcher(8s tick),全量枚举 Ghostty terminal 的 `(id, working directory, title)`,按 **title 含 `external_session_id` 前缀**(主键)/ cwd 兼容(兜底)把 claude-code/codex session 绑定到稳定 terminal UUID。覆盖单次 capture 抓不到的冷启动缺口;title 主键命中可**纠正** cwd first-match 造成的错绑(cwd 兜底仍只在空位写,不覆盖)。仅在有相关 session 时才枚举,枚举期不持 store 锁。

### Changed

- **授权后端从 Lemon Squeezy 迁到 Creem**:`open_purchase_url` 跳 Creem checkout;license activate/validate/deactivate 走 Creem API(`creem_test_*` key → test-api.creem.io,否则 api.creem.io)。ProductKind 收敛为 `MonthlySubscription` + `Lifetime` 两档。编译期 env 改为 `POKEPOKE_CREEM_*`(purchase URL / API key / 两个 product id);全留空仍可 build、可启动,仅 license 功能不通。
- **license 落盘位置明确为本地文件**:`~/.pokepoke/license.json`(0600 文件权限,**不走 macOS Keychain**),遗留文件升级时自动迁移并补 chmod。
- **WebView CSP 从 null 收紧到白名单**:杜绝任意外链/inline 注入面。
- **日志文件超 5MB 自动轮转**:避免常驻进程日志无限增长。

### Fixed

- **多设备授权停用按实例状态判定**:修「停用需点两次」的 bug —— 以实例 deactivated 状态而非整单状态判定。
- **validate 对 expired/disabled 状态降级**:授权过期/停用时正确回落 Free,不再卡在 Pro。
- **Ghostty 跳转命中加固**(brief `20260629-ghostty-jump-reconcile`):修复三类症状 —— 跳到 Ghostty 没切对 tab / 焦点没落位需手动点一下 / 多 tab 同 cwd 跳错。`focus_ghostty` 的 L1(按 id)与 L2(按 cwd)脚本改为先 `window→tab→terminal` 三级定位,再 `activate window`+`select tab`+`focus`+`delay`,L1 额外做 focus 后读回 `focused terminal` 验证 + 重试(≤3 次),焦点确实落位才算成功。根因:本机 Ghostty `working directory` 常停在家目录(OSC7 未持续上报),旧单次 capture 的 cwd 校验恒失败导致 terminal_id 永不落库、跳转恒退化到 L2 cwd first-match(既无验证又会撞车)。

## [0.0.8] - 2026-06-25

### Added

- **Lemon Squeezy Pro 授权**:Settings 新增 License 区,支持 activate / validate / deactivate;`open_purchase_url` 跳转 LS checkout 完成购买。HTTP server 侧每日 validate 一次;新增免费层每日通知配额,超额后新 popup 被抑制(`license_allows_new_disturbance`,一条新的 popup 抑制规则)。新增 Tauri commands `get_entitlement` / `activate_license` / `validate_license` / `deactivate_license` / `open_purchase_url`。授权状态持久化到 `~/.poke-poke/license.json`(不走 macOS Keychain)。
- **Pro 子类区分**:新增 `ProductKind::MonthlySubscription`(月度订阅),与旧版月付 `Monthly`、买断 `Lifetime` 并列;新增编译期 env `POKEPOKE_LEMON_MONTHLY_SUBSCRIPTION_VARIANT_ID`(default `1826357`)。前端按子类显示不同标签;持久化的 `"monthly"` 仍向后兼容反序列化为 `Monthly`。
- **Claude Code transcript 中断提醒**:transcript 出现未恢复 `[Request interrupted by user]` marker 时,以中性 `pending` 提醒用户返回查看/继续。watcher 只扫描 `running` 的 Claude Code session,复用 `/notify` 链路触发 popup/sound。

### Changed

- **购买入口收敛为单一「升级 Pro」按钮**:`open_purchase_url` 去掉 `kind` 参数,两个编译期 env(`POKEPOKE_LEMON_MONTHLY_URL` + `POKEPOKE_LEMON_LIFETIME_URL`)合并为单个 `POKEPOKE_LEMON_PURCHASE_URL`;月度/买断的选择改在 LS checkout 页完成。
- **仓库转为闭源专有软件**:删除 MIT LICENSE 与 CONTRIBUTING.md,README 改为专有声明(不再允许 redistribution);开源贡献流程(公开 mirror PR 回放)退役。

### Fixed

- **Pending 永远 eligible,不再被同 message 抑制**:旧 `is_stage_popup_eligible` 对 Pending→Pending 用"同 message 抑制"把 hook retry 吞掉,导致 agent 卡在 Pending 等批准/输入时用户错过首次 popup 后无声卡死。现改成 Pending 始终 eligible,频率改由 `PopupCooldown`(3s)+ `AckedStages`(对 Pending 跳过,认领不等于已授权)兜底。
- **Ghostty capture / cwd 校验放宽**:shell integration 未上报 OSC7 时 working directory 整体不可用,旧逻辑把 cwd 空误判为"抓错 tab"导致 terminal id 不落库 / 跳转误走 L2。retry 场景下 cwd 空允许写入(capture reason 区分 `session_start` vs `retry-*`);focus 校验区分"tab 不存在"(invalidate)与"tab 在但 cwd 不可读"(信任已落库的 stable id,直接 L1 跳)。
- **Lemon Squeezy variant_id 解析修正 + product_kind 不再被清空**:variant_id 从响应顶层 `meta` 对象读取(原误读 `license_key.variant_id`);`validate_license` 在 provider 未返回 kind 时保留上次已知 good kind,避免 API 短暂故障把 UI 标签退化成 Free。

---

## [0.0.7] - 2026-06-01

### Changed

- **Ghostty `terminal_id` capture 从 SessionStart-only 扩展到多事件 retry**(brief `20260527-ghostty-capture-retry.md`,推翻 b20520 的 Non-goal "不重 capture")。SessionStart capture 仅 72% 命中(多 tab 并行 / cc daemon resume / cwd-mismatch reject),后续在 UserPromptSubmit / Notification / Stop 三个事件上对 terminal_id 仍为 None 的 ghostty session 补一次幂等 capture(`set_terminal_id_if_unset` 保证已写入的永不覆盖),预期把 L1 命中率拉到 ~95%。**对前向跳转用户(b20520 受益群体)是纯收益**:同 cwd 多 session 跳错 tab 的概率显著降低。对反向通道(独立 brief 未立)是硬前置 —— 后者要求 L1 命中率 ≥95% 才能不误注入到错 tab。hook 端 pokehook `terminal_app` 字段从 SessionStart-only 扩展到所有 cc/codex 事件(Cursor 事件保持不带,Cursor 一般不在 Ghostty)。server 端 `spawn_ghostty_capture` 提为共享 helper,SessionStart 首次和 retry 都调它,日志带 `reason=session_start | retry-user_prompt_submit | retry-notification | retry-stop` 分桶便于真机统计。
- **`bot_outbound.include_message_body=true` 推送内容从合成 placeholder 升级为 transcript 摘要 + 修 LastFailed 渲染空 `**消息**:` 行**(配套 0.0.7 飞书推送 MVP 落地)。0.0.7 飞书推送上来时,outbound 推送 body 走的是 pokehook 合成的 placeholder("Session completed" / "" / "Working...\n{cwd}") —— 用户开 `include_message_body` 也只看到这些占位文案;LastFailed 因 message 为空字符串,卡片直接渲染出空 `**消息**:` 行(用户感知 = 推送残缺)。本次接入 cc/codex hook payload 自带的 `transcript_path`(指向 `~/.claude/projects/.../{session}.jsonl`),outbound 启用 `include_message_body` 时按 status 抽:LastFailed 优先 `isApiErrorMessage` 文本(真 API 错误体),Idle 取最近 3 个 tool_use + 最后一段 assistant text,Pending 在 Notification 原文之后追加 2 个 tool_use 作上下文,Running 跳过(transient)。每条 200 字符 + 总 500 字符双截断;Bash 取 cmd 首行、Edit/Read 取 file basename(完整路径不漏)。**`include_message_body=false` 时根本不读 jsonl 文件**(强隐私 hardgate,强化原 fingerprinting 边界)。pokehook 5 个 handler(SessionStart/UserPromptSubmit/Notification/Stop/StopFailure)透传 transcript_path,snake/camel 双键兼容;Session + NotifyRequest schema 加 `transcript_path: Option<String>`(`#[serde(default)]` 兼容老 sessions.json,upsert 时 Some 覆盖 / None 保留旧值,防后续事件漏字段把已知路径清掉)。v1 仅 cc(jsonl schema 写死),codex/cursor 透明降级到原 placeholder 路径,后续 brief 补完。新增 `bot_outbound/transcript.rs` 模块 + 14 个 fixture 单测覆盖 happy/api_error/empty/malformed/clip/tail-truncation;5 个 bot_outbound 新单测 + 1 个 sessions 端到端单测。

### Added

- **移动端推送(MVP):飞书自建应用机器人 outbound**(brief `20260527-mobile-push-feishu-dingtalk`,**rescope 2026-05-28**)。Settings 加"移动端推送" section,用户在飞书开放平台注册自建应用,填 `app_id` / `app_secret` / 自己的 `open_id` / endpoint(国内飞书 vs 海外 Lark),桌面 popup 实际创建时(`popup_created==true`)异步通过飞书 IM API 给指定用户**私聊**一份提醒卡片。**比群机器人 webhook 接入步骤多** —— 但用户体验显著更干净(无空群骚扰,飞书机器人直接 DM,适合开发者用户)。两层 gate:第一层硬门禁完全复用 popup 抑制链(`already_focused` / `recently_visited` / `acked_stages` / `popup_cooldown` / `notifications_enabled` / refresh-only 都不推);第二层 `BotOutboundTrigger` ∈ `TerminalUnfocused`(默认)/ `Always` / `PendingOnly`,**`Always` 不绕过第一层**。token cache:`tenant_access_token` 2h 有效,按 `app_id` 共享缓存,提前 5 min 刷新,多 provider 同 app_id 不重复刷。默认 `include_message_body=false`(不送 session.message 原文给飞书);默认带 status/source/workspace_basename/PokePoke/时间/失败原因(仅 LastFailed);**永不**带 task_id/session_id/完整 workspace path。默认节流 60s/同 session(测试按钮 bypass)。卡片**严禁**任何 action/button 元素(`elements[*].tag` ∈ `{div, hr}` + 递归无 `action`/`button`),杜绝"可远程控制"暗示。日志强制走 3 个脱敏 pure fn(`redact_bot_outbound_error` / `safe_webhook_host` / `short_provider_id`),App Secret / open_id / token 不进 log。失败只 log 不重试,`OutboundFailureLogLimiter` 每 10 次失败只 log 1 次防刷屏。provider 用 uuid v4 stable id,删除/重排不串测试目标。首次启用弹一次性 confirm "消息会经飞书服务端转发"。新增 settings 字段 `bot_outbound`(向下兼容,老 settings.json 升级后 enabled=false 默认安全);新增 Tauri command `test_bot_outbound`(按 provider_id);后端模块 `bot_outbound::{redact, throttle, token_cache}`,38 个单测覆盖 token cache 状态机 / payload schema / 字段白名单 / redaction / throttle / log limiter / endpoint URL。**不支持钉钉**(对个人开发者门槛过高,Hermes / openclaw 等内部工具的"零配置"实质是公司基础设施)。**inbound**(回复 agent / 卡片按钮 / 公网回调)是独立 brief 范围,本 MVP 命名预留(`bot_outbound::OutboundBotProvider`)但不实现一行入站代码。配套接入教程见 `docs/guides/mobile-push-setup.md`。

---

## [0.0.6] - 2026-05-26

### Fixed

- **Pending → Pending 抑制吃掉新内容 Notification**(2026-05-25 实测复现):用户 dismiss 上一个 Pending popup 后,cc 又 fire 一条**新内容**的 Notification(如"plan 等批准"),被 0.0.3+ 的"连续 pending 抑制"规则吞了 —— 用户没收到提醒,session 只是在 sessions.json 里默默 updated。原规则一刀切抑制 Pending → Pending(防 cc 同 message 反复轮询),现在改成 **message 内容跟前一次相同才抑制,不同即视为新需求弹一次**。`UpsertResult` 加 `prev_message: Option<String>` 把旧 message 冒给 `should_show_popup`,后者新增 `new_message + prev_message` 入参做差异判断。覆盖 cc Notification "需要审批 plan" 这种典型用户痛点;cc 同 message 的 keep-alive 轮询继续被挡住。配套单测:`should_show_popup_for_pending_to_pending_different_message`(新行为正例)+ `should_show_popup_for_pending_to_pending_same_message`(原行为反例)+ prev_message=None 边界。
- **Ghostty 跳转稳定跳错** + **历史脏 terminal_id 自愈**(2026-05-25 实测复现):用户在多 cc workspace 场景下,popup ⌘N 跳转**稳定**落到错误 Ghostty tab,且不同 workspace 的 session 也会跳到固定错位置(不是随机)。两个根因连环:(1) `cwds_compatible`(eec181f 引入的 capture 校验)**允许反方向前缀**(`session_workspace.starts_with(captured + "/")`),导致 capture 时如果 Ghostty 前台是 home shell(`/Users/edy`),**任意 workspace 的 session 都被它套上家目录 tab 的 id** —— `/Users/edy` 是几乎所有路径的祖先。session.json 里 `cc-c252379b (/Users/edy)` 跟 `cc-de925cce (/Users/edy/.../star-maker)` 撞同 `terminal_id` 就是这条路径打中。(2) `focus_ghostty` L1 by id 写死信任 stored id 不验证,历史脏数据 + (1) 的 capture race 一旦污染就**稳定地把所有 click 跳到错 tab**。两件修法:(a) `cwds_compatible` 去掉反方向前缀,只允许 captured 等于或在 session 子目录里;(b) `focus_ghostty` L1 增加"先 query 该 id 当前 cwd → 用 `cwds_compatible` 验,不通过当 miss + 反向通过 `FocusGhosttyResult.invalidate_id` 通知 caller";新增 `SessionStore::clear_terminal_id_if_matches(session_id, expected)`(防 race 只清匹配的)在 `focus_session_terminal` / `open_session_source` 收到 invalidate 时调用 —— **已经被污染的 session 第一次 click 时自动清掉脏 id,下次走 L2 cwd 兜底**,不用手动改 sessions.json。新增单测:cwds_compatible 反方向 reject 一例,sessions clear 4 例(missing / currently-none / mismatch-no-op / matched-clears)。(v0.0.5 实测复现):用户聚焦在 Ghostty 终端时,popup 仍然弹出,本应被 `already_focused` 闸门 suppress。根因:server 端 Ghostty terminal_id capture gate 太严,只在 hook 明确报 `terminal_app="ghostty"` 时触发。但 cc 在 resume / compact / background reconnect 等场景从 daemon spawn hook 时,**所有 Ghostty 信号 env(TERM_PROGRAM / TERM / GHOSTTY_*)都可能丢光**,24a3d41 多 env 兜底也兜不住,terminal_app 上报为 None → 不 capture → terminal_id 永 None → `is_focused` 的 Ghostty 分支需要 terminal_id 来比,缺则永远 false → popup 不被 suppress。**两段修法**:(a) gate 放宽 —— `terminal_app` 缺失 OR 显式 `ghostty` 时都 try capture,只显式标记非 ghostty(iterm.app / apple_terminal / vscode / cursor 等)才跳过;capture 函数自身已有 `is_app_running("Ghostty")` 守门,Ghostty 没在跑直接返 None。(b) **cwd 校验** —— gate 放宽会引入 opportunistic capture 抓到无关 Ghostty tab id 的风险(cc resume 后台 fire + 用户切走 / iTerm2 用户 Ghostty 有无关 tab 前置)。`capture_ghostty_focused_terminal_id` 改成同时返回 `(id, cwd)`,server 端比对 `cwd` 跟 `session.workspace_path` 的前缀关系(允许 cc 启动后 `cd` 子目录,也允许 session 指定子目录而 tab 在根),不兼容直接拒写。守住 click-jump L1 不会跳错 tab + `is_focused` 不会误 suppress 别处。配 8 个单测覆盖 exact / trailing-slash / sub-dir 双向 / 无关路径 / sibling 误前缀 / 空 / 根目录边界。

---

## [0.0.5] - 2026-05-21

### Added

- **Ghostty 1.3+ 终端跳转兼容**(brief `20260520-ghostty-terminal-focus`):点击 popup / ⌘N 跳转现在支持 Ghostty。SessionStart 事件到达 server 时,如果 hook 上报 `terminal_app="ghostty"`,异步走 `osascript -e 'tell app "Ghostty" to id of focused terminal of front window'` 抓 stable terminal id 存到 session。后续 focus 走三层兜底:**L1** 按 id 精准跳(主路,即便多 tab 同 cwd 也准)→ **L2** 按 working directory first-match → **L3** 仅 activate Ghostty。Ghostty 未运行 / < 1.3 没 sdef / 用户瞬间切走 tab 时全自动 fall back。首次跳转会触发 macOS TCC 一次性"允许 PokePoke 控制 Ghostty"授权,与 iTerm2 / Terminal.app 体验对齐。
- **Ghostty 适配补全:popup 弹屏定位 + 自动关闭**(task #54/#55):跟跳转一起构成 Ghostty 完整支持。`monitors::terminal_monitor` 多加 Ghostty 分支(按 terminal_id 用 AppleScript 遍历 `windows × tabs × terminals` 找窗口 bounds),`popup_screen_strategy=Terminal` 时 Ghostty session 也能正确弹到所在屏,不再 fallback 到光标屏。`is_terminal_session_focused` 同步加 Ghostty 分支(按 terminal_id 匹配 `id of focused terminal of selected tab of front window`),focus-watcher 改成"任一 tty/terminal_id 在就 spawn"——Ghostty session 现在也能在用户聚焦回去时 1.5s 内自动关 popup,不用等下一轮 Running 事件。覆盖原 task #56 "听到声音没 popup" 假设 (A) 同根因路径。
- **Ghostty 识别多 env 兜底**(实测踩坑):pokehook 的 `terminal_app()` 原本只读 `TERM_PROGRAM`,但 cc 在 resume / compact / background reconnect 等场景从 daemon 进程 spawn hook 时,`TERM_PROGRAM` 经常丢(env 来自 daemon 不是用户原终端),导致 server 收不到 `terminal_app="ghostty"` → 不触发 capture → terminal_id 永远 None → click 跳转走 L2 cwd first-match,**多 tab 同 cwd 时就偏到错的 tab**。补 fallback:`TERM=xterm-ghostty` / `GHOSTTY_RESOURCES_DIR` / `GHOSTTY_SHELL_FEATURES` 三个 Ghostty 专属 env 任一在即识别为 ghostty(这些 env 由 terminfo / Ghostty bundle 设,不易被 daemon spawn 剥)。优先级:TERM_PROGRAM 直接命中 > Ghostty fallback。配 6 个单测覆盖每条路径 + case-insensitive。
- **Updater popup 体感优化**(brief `20260521-updater-popup-ux`):升级提示 popup 加 release notes 折叠区(默认折叠成"查看更新内容"链接,展开 ~120px 滚动框)+ 下载/安装进度条。【立即升级】拆成两步 command:`download_update_with_progress` 通过 `update-progress` event 推下载进度(前端 rAF 合并避免重渲染卡顿),`install_cached_update` 走安装+重启(indeterminate 文案)。任一步失败前端按钮变成"重试"可重入。设置面板「立即更新并重启」按钮仍走旧 `install_update`(无进度需求)。`scripts/gen-latest-json.sh` 顺带从 CHANGELOG 当前版本节抽 notes(zh + en)写入 `latest.json.notes`,客户端 `get_pending_update_info` command 读出来给 popup 展示;旧的写死 `"See CHANGELOG.md for details."` 弃用。新增设置项:无。新增 i18n keys:`popup.update_downloading_pct` / `popup.update_downloading_bytes` / `popup.update_installing_restart` / `popup.update_retry` / `popup.update_notes_show` / `popup.update_notes_hide` / `popup.update_downloading`。

---

## [0.0.4] - 2026-05-20

### Added

- **Codex `PermissionRequest` 事件订阅**(brief `20260520-hook-coverage-expansion`):Codex agent 跑命令需要用户审批时,与 Claude `Notification.permission_prompt` 同语义弹 popup,session 升 Pending 状态。`--install-codex` 写入 4 个事件(原 3 个 + PermissionRequest),`--check-codex` 校验 4 个;payload 含 `tool_name` 字段时消息体显示 `Awaiting approval: <tool_name>`,缺失退化为通用 `Awaiting approval`。**注意 Codex 0.131+ hook trust 门禁:首次接入后需 TUI 里 `/hooks` 信任一次**。
- **`StopFailure` 补齐 2 类 reason 文案**(brief `20260520-hook-coverage-expansion`):Claude Code `StopFailure` 官方 9 类 matcher 中漏掉的 `oauth_org_not_allowed` 和 `model_not_found`,前端 `KNOWN_REASONS` 白名单 + 中英 i18n 补齐,落在这两个 reason 上的会话不再被打回 `unknown`("API 错误"),用户能看到具体类型。

### Fixed

- **0.0.3 孤儿 NSPanel popup**(brief `20260520-nspanel-orphan-destroy-v2`):popup 关掉后 macOS 屏上残留幽灵 NSPanel,鼠标 hover 显示菊花、全局快捷键 `⌘N` 编号错位。根因是 `close_popup_by_label` 用 `win.close()`(软关闭)在某些时机被吞成 orderOut,NSPanel 对象未真正释放。改为 `win.destroy()`(硬销毁);仍走 `panel.to_window()` 确保 ObjC class 已改回 NSWindow,绕开 v1 fix 因跳过 `to_window` 撞 selector mismatch 触发 NSException SIGABRT 的坑(stash@{0} 已弃用)
- **⌘N 跳转误关全部 popup**(brief `20260520-nspanel-orphan-destroy-v2`):同时有 2+ 个 popup 时,按 ⌘1 跳转 popup1,popup2/3/... 也一起消失。根因是 `PopupWindow.tsx` 用 Tauri 全局 `listen("popup-shortcut-trigger", ...)`,无视后端 `app.emit_to(label, ...)` 的目标过滤 —— 每个 popup 都收到事件,都执行 `handleClick`(focus_session_terminal + close_popup_window)。改为 `getCurrentWebviewWindow().listen(...)` window-scoped,严格只收投向本窗口 label 的事件

### Changed

- **Codex feature flag 键名迁移**(brief `20260520-codex-feature-key-migration`):`--install-codex` 现在写 canonical 键 `[features].hooks = true`(原 `codex_hooks` 是 deprecated alias);`--check-codex` 同时识别两个键(`hooks || codex_hooks`),老 PokePoke 装过的用户升级后不会被误报"未启用";`--uninstall-codex` 两个键都不动(沿用既有"feature flag 可能是用户独立偏好"哲学)

---

## [0.0.3] - 2026-05-19

### Added

- 设置项 `update_auto_check: bool`(默认 true)+ `update_skip_version: Option<String>` —— 启动 5s 后台首检 + 每 24h 一次 updater 巡检,命中新版且未跳过则弹"升级提示" popup。popup 上三个操作:【立即升级】/【跳过本版】/ 右上角 `×`(暂不决定,下一轮 24h 巡检会再弹),**强制不受 `popup_auto_close_secs` 约束**(updater 提示不能被 5s/30s 吃掉)。dismiss 走 `dismiss_update_popup` command(不写 `update_skip_version`),跟「跳过本版」语义区分。
- 全局快捷键 `⌘1` ~ `⌘9` + `⌘Enter` —— 屏上有 session popup 时按下等价于鼠标点击对应位置 popup(跳转到该 session 的终端 + 关 popup)。`⌘Enter` 是 `⌘1` 的别名。popup 右下角显示 `⌘N` 角标提示当前编号。
- popup_list 空时**全部反注册**全局快捷键,把 `⌘Enter` 等高频组合键还给其他 app;有 popup 时按需 register `min(N,9)+1` 个。

### Fixed

- **升级 popup label 含 `.` 被 Tauri reject**(unique-label refactor 引入的暗坑,真实 updater 命中新版会让整个 popup 创建失败):Tauri 的 webview label 只允许 alphanumeric + `-`/`_`/`:`/`/`,semver 必带 `.`,我们的 `popup-u-{token}-{version}` 直接撞墙。修复:新增 `version_to_label_token` helper 把 `.` / `+`(semver build metadata) 编码成 `_`,`update_popup_label` / `find_update_popup_label` 对称编码;`update_popup_label_is_tauri_safe_for_semver` 单测钉住契约,覆盖 `9.9.9` / `1.0.0-rc.1` / `1.0.0+build.42` 三种 semver 形态
- **updater 巡检任务的几个体感坑一次性收口**(评审 H1/H2/H3a/H4):
  - **toggle 改动最多 24h 才生效** → 改成 60s tick + `next_check_at` 状态控制,toggle false→true 最多 60s 生效(而非次日)
  - **check 失败拖 24h 才重试** → Err 路径 15min 短 retry,开机无网首检失败的笔记本场景不再丢一整天
  - **install_update 内部重复 check()** → 后台巡检命中后 Update 句柄缓存到 `PendingUpdate: Arc<Mutex<Option<Update>>>`,install_update command 直接复用,避免多一次网络 RTT + 远端 release 撤回时的"no update available"边界错误
  - **install 失败前端静默** → catch 错误后写 `errorMsg` state,popup 上以红字「升级失败,稍后再试」展示,hover 看原始错误细节
- **popup 左下角时间改用 `session.updated_at` 而非 `created_at`** —— 老 session 来个新事件不再显示"4 小时前"造成"老 popup 复活"错觉;跟 panel session-time 语义对齐
- **show_popup 内 same_screen + visual_index + push 合到一次 popup_list lock**(评审 M2):之前是 `calculate_position` 锁 → `session_popup_count` 锁 → 后续手动 push 锁,并发两次 show_popup 之间 visual_index 可能算错一帧(URL hash baked 进去的 ⌘N 角标值落后);`emit_popup_shortcuts_updated` 50ms 内会通过 store-emit 纠正前端角标,但首帧不一致。重构后单一 `reserve_slot_and_position` 在同一锁内原子完成读 + push,首帧就准
- **pokehook 输出的 popup `title` 字段去掉 `<source>:` 前缀**(`"Claude Code: poke-poke"` → `"poke-poke"`):popup 右上角已有来源 tag(CLAUDE-CODE / CURSOR / CODEX),前缀重复;同时清理掉只剩这一处用途的 `Source::label()` 死代码
- **popup label 改用「每实例唯一」格式根除跨实例 race**(根因修复 —— 0.0.2 / 0.0.3 反复出现的"听到提示音没看到 popup"由此终结)。旧设计 `popup-{session_id}` 同 session 复用 label,违反 Tauri 官方 `WebviewWindowBuilder` 文档「unique labels per instance」建议;表现为:
  - 0.0.2 auto-close timer 醒来按 label 判存在 → 误关同 session 后继 popup(用 token 校验修过)
  - 0.0.3 focus-watcher 一模一样的姐妹 bug(用 token 校验修过)
  - close_popup 主线程 dispatch 排队期间若有同 label 新 popup 进入 Tauri 注册表,destroy 可能击中新实例(理论 race,实测受 main-thread FIFO 保护未观察到,但同 label 复用本身就是反模式)
  
  改造:label 新格式 `popup-s-{token}-{session_id}` / `popup-u-{token}-{version}`。POPUP_INSTANCE_COUNTER 给每个 popup 实例发独一无二的 token,**任何两个 popup 永远不会共享 label**。配套:
  - `close_popup(session_id)` 内部走 `find_session_popup_label` 反查;`mark_update_skipped(version)` 内部走 `find_update_popup_label` 反查
  - auto-close timer / focus-watcher 捕获自己创建时的 `label`,触发时直接 `close_popup_by_label(my_label)`,从源头不可能动到后继实例
  - 前端 `popup-` 前缀替换改成正则 `popup-s-\d+-(.+)` 提 session_id
  - Tauri label 命名空间不再有任何复用 → Tauri 内部 label 注册表的 close-then-recreate race([#9353](https://github.com/tauri-apps/tauri/issues/9353) 同类)对我们彻底消失

### Changed

- popup URL hash 多两个参数:`kind=session|update`(前端按类型路由不同模板)+ `n=<0-based 视觉编号>`(session popup 用于 ⌘N 角标显示);跨屏全局统一编号。
- 设置面板 About 段加 `update_auto_check` toggle;窗口高度从 990 → 1040 以容纳新一行。

---

## [0.0.2] - 2026-05-18

### Added

- 设置项 `popup_auto_close_secs: Option<u32>` —— popup 显示 N 秒后自动关闭。`None`(默认)= 永不,保持 0.0.1 行为不变;前端设置面板新增「自动关闭时长」下拉,可选 永不 / 5s / 15s / 30s / 1min / 3min。后端不做白名单,任意 `>0` 的整数都尊重(方便手改 `settings.json`)。

### Fixed

- macOS `close_popup` 销毁链(`panel.to_window()` 内部 `object_setClass` + `win.close()`)历史上被多种后台线程直调(tokio worker / focus-watcher / Tauri command / 自动关闭 timer),非主线程改 AppKit obj-c class 是 UB —— 表现为 `popup_list` 已清干净但屏上 panel 还在,React 端靠 store-emit 把内容刷成 Running 状态,看起来像 popup 永不消失。`close_popup` 内部下沉 `run_on_main_thread`,所有调用方自动安全。
- 自动关闭 timer 用 `(label, token)` 双键匹配 popup 实例,避免同 session 旧 timer 误关后继新 popup(label 复用 `popup-{session_id}`,只看 label 会跨实例错关,体感"popup 闪都没闪过")。

### Changed

- 日志时间戳从 UTC + `Z` 改为系统本地时间 + `+HH:MM` 偏移(如 `2026-05-18T16:13:00.123+08:00`),与终端时间直接对账。

---

## [0.0.1] - 2026-05-12

首次正式发布。下列为**首发态行为契约**。早期开发期反复变更的中间态(transport TCP↔UDS 来回、`endpoints.json` 多实例发现表、`~/.pokepoke/locks/` 文件锁、`_pokepoke` 受管标记、`install-meta.json` 元数据、调试模式开关、全局快捷键、Codex 一键接入、开机自启…)不再追溯,过程详见 git log。

### 接入 / 安装

- Claude Code 一键接入:写入 `~/.claude/settings.json` 的 `SessionStart` / `UserPromptSubmit` / `Notification` / `Stop` / `StopFailure` 五个事件;Disconnect 走对称卸载。
- Cursor 一键接入(用户级全局):写入 `~/.cursor/hooks.json` 的 `sessionStart` / `beforeSubmitPrompt` / `stop` / `sessionEnd`。
- Codex CLI:**不**管理 `~/.codex/hooks.json` / `~/.codex/config.toml`(用户自己维护);事件走 payload `turn_id` 强信号 + `--source=codex` arg 识别,panel / popup 仍正确显示。
- Hook 命令格式:`~/.local/bin/poke-hook --source=<cc|codex|cursor>`,install-time 钉死 source。
- 启动期 hook binary 自动部署:bundle 内 `Contents/MacOS/pokehook` 强制覆盖到 `~/.local/bin/poke-hook`(每次启动幂等,~10ms),协议升级零摩擦,**无需** `pnpm install:hook`。
- 启动期 hook 配置漂移自检:command 字符串过期(缺 `--source=<x>` 后缀)时 emit `hook-drift-detected` → panel 顶部显示告警 + 按 source 提供「一键修复」按钮。
- 写前 snapshot:每次改写 `~/.claude/settings.json` / `~/.cursor/hooks.json` 之前先复制到 `~/.pokepoke/snapshots/<source>-<ts>.json`,每 source 池滚动保留最近 5 份(write-only,用户手动 `cp` 复原)。

### Changed - 2026-04-27

- Codex CLI 恢复为 PokePoke 受管接入端:托盘 Connect / Disconnect / Check 写入 `~/.codex/hooks.json` + `~/.codex/config.toml`,并启用 `[features] codex_hooks = true`。
- 启动期 hook drift 自检与 panel 一键修复再次纳入 Codex;修复会走 `--install-codex`。
- 写前 snapshot 覆盖 Codex 双文件:`codex-hooks-<ts>.json` 与 `codex-config-<ts>.toml` 分池保留。

### Source 判定

`hook.rs::detect_source` 三层优先级:
1. payload 独占字段(强信号):`workspace_roots` / `workspaceRoots` → Cursor;`turn_id` → Codex
2. install-time 钉死的 `--source=<x>` arg
3. 兜底 → ClaudeCode

强信号优先于 arg 是为 Cursor *Third-party hooks* 双触发场景:Cursor 读到 `~/.claude/settings.json` 的 `--source=cc` 命令,但 payload 实际是 cursor 的(带 `workspace_roots`),强信号 override 才不会误归类。

### 通信层

- Hook ↔ server 走固定路径 UDS `~/.pokepoke/sock`,文件 0600 / 父目录 0700。
- 单实例语义:启动时若 sock 已存在 → 探活 connect → 连得通则对等实例存活,本进程退出;连不通则 stale unlink 后重新 bind。
- 不开任何 TCP 端口,不与外部服务通信。
- Hook 端手写 HTTP/1.1 over UnixStream,read/write 各 2s 超时;server 不在线时静默丢事件 + PrintOnDrop 输出合法 `{}` 满足 hook host 契约。

### Session 状态机 + 探活

- 状态:`running` / `pending` / `idle` / `last_failed`(`success` / `failure` 作为 serde alias 向下兼容老数据)。
- `event_type` 区分「状态更新」与「会话彻底结束」(`session_end` 直接 remove,不入四态)。
- 探活:5s 周期对所有 session 一次 ps 探查,连续 2 次 miss 才 reap(grace 10s,防 ps 偶发误判)。按 source 分层:claude-code / codex 走 `ps -t <tty> -o comm=`,cursor 走 `pgrep -x Cursor`(粒度到 app 级)。
- 启动时无条件清空 panel(等 agent 重新报到),避免 stale row 残留。

### 事件去重

`http_server::DedupeWindow` 按 `(task_id, signature)` 100ms 内存窗口判定重复,命中返回 `200 + {"deduped": true}`,不动 store / 不弹 popup / 不放音。第三方 hooks 双触发场景(Cursor 同时读 `~/.claude/settings.json` 与 `~/.cursor/hooks.json`)由此兜住。

### 设置项

- `alert_sound`: 任意系统音 / `mute`(默认 `system:Glass`)
- `locale`: `zh` / `en`(默认 `zh`)
- `notifications_enabled`: 全局提示总开关(默认 `true`)
- `popup_position`: 屏幕 4 角任选(默认 `top-right`)
- `tray_icon_style`: `colored` / `white`(默认 `colored`)

老 `settings.json` 残留的已删字段(`auto_start` / `panel_shortcut` / `dev_mode`)被 serde 忽略,升级零成本。

### Popup

- macOS 走 NSPanel(`PokePopupPanel`,nonactivating + can_become_key=false),不抢焦点。
- 出现位置跟随 `popup_position` 设置,堆叠方向与 CSS slide-in 动画方向跟随选定的角。
- 失焦 / 点击「跳转」/ panel 隐藏 / `event_type=session_end` 都触发关闭。
- 点击跳转:走 `decide_focus_target` 纯函数分档 — 有 tty → AppleScript 精确定位 iTerm2 / Terminal tab;Cursor GUI 无 tty → `cursor <workspace_path>`;其它无 tty → `open <workspace_path>` 打开 Finder。

### Tray / Panel

- 多屏定位:`pick_monitor_at_point` 用物理坐标遍历 `available_monitors()` 找点击所在屏,在该屏 logical 坐标系内放 panel(不再总回主屏)。
- Panel 头部右上 ⏻ 关机按钮 → `quit_app` Tauri command → `app.exit(0)`。

### POKEPOKE_DEBUG

设 `POKEPOKE_DEBUG=1`(或 `true`/`yes`/`on`)在启动时启用 DEBUG 级日志落 `~/.pokepoke/pokepoke.log`,无运行时切换通道。

### Bundle Identifier

锁定 `com.pokepoke.PokePoke`(此后**不会**再改,改了升级路径会断)。

### 手动残留清理(可选)

旧版本机器上以下文件 / 目录代码不主动清理,如需可手动 `rm`:

```bash
rm -rf ~/.pokepoke/sockets ~/.pokepoke/endpoints.json ~/.pokepoke/endpoints.lock
rm -rf ~/.pokepoke/locks
rm  -f ~/.pokepoke/install-meta.json
```

---

## 历史

尚无正式发布版本。
