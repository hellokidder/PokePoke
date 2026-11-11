// PokePoke DeepSeek Harness integration — host half.
// pokepoke-plugin-marker
// Do not edit by hand; re-run Connect from the PokePoke tray to regenerate.
//
// DSH loads this file as a cordis plugin inside its Host process. It forwards
// session lifecycle events to the local PokePoke app through the poke-hook
// binary, and hosts a small unix socket that lets PokePoke ask the renderer
// half (./client.js) to open one exact conversation.
//
// Everything here must stay dependency-free: DSH resolves this module by
// absolute path, so only Node built-ins are safe to import.
import { spawn } from 'node:child_process'
import {
  appendFileSync,
  chmodSync,
  existsSync,
  mkdirSync,
  renameSync,
  statSync,
  unlinkSync,
} from 'node:fs'
import { createServer } from 'node:net'
import { homedir } from 'node:os'
import { join } from 'node:path'

const POKE_HOOK = '__POKE_HOOK_BIN__'
const HOME = homedir()
const RUN_DIR = join(HOME, '.pokepoke')
const SOCKET = join(RUN_DIR, `dsh-${process.pid}.sock`)
const LOG = join(RUN_DIR, 'dsh-plugin.log')
const DEBUG_FLAG = join(RUN_DIR, 'dsh-debug')
const LOG_MAX_BYTES = 5 * 1024 * 1024
const OPEN_TIMEOUT_MS = 8000
// turn/end 只说明"这一轮结束了",不说明"这个会话不再自己往下跑":goal 会自动续轮,
// background subagent 跑完后还会把消息 splice 回父会话开新 turn。实测 2026-11-10
// (session ff40ff3b)父会话 turn/end 时两个子会话还在跑,12 分钟后才 splice 回来,
// 任何固定去抖窗口都盖不住。所以改用 DSH 自己的状态做「静默门」(见 settleStop)。
const SETTLE_POLL_MS = 250
const SETTLE_QUIET_MS = 1200
// DSH 卡在某个永不消失的 running 子会话时的兜底,避免这条 session 永久失去完成通知。
const SETTLE_MAX_MS = 60 * 60 * 1000
// 回落路径:拿不到 DSH 内部服务(版本漂移 / 会话已卸载)时,保持旧的固定去抖。
const STOP_DEBOUNCE_MS = 1200

// 排障开关:DSH 的进程环境用户改不了,所以除环境变量外再认一个标记文件
// (`touch ~/.pokepoke/dsh-debug`,rm 关闭),每次写日志时现查,无需重载插件。
function debugEnabled() {
  if (process.env.POKEPOKE_DSH_DEBUG === '1') return true
  try {
    return existsSync(DEBUG_FLAG)
  } catch {
    return false
  }
}

function rotateIfNeeded() {
  try {
    if (statSync(LOG).size > LOG_MAX_BYTES) renameSync(LOG, LOG + '.1')
  } catch {}
}

function debug(message) {
  if (!debugEnabled()) return
  try {
    rotateIfNeeded()
    appendFileSync(LOG, `${new Date().toISOString()} [${process.pid}] ${message}\n`)
  } catch {}
}

/** Forward one normalized event to PokePoke through the poked hook binary. */
/**
 * 串行转发队列。
 *
 * 每条事件都会 spawn 一个 poke-hook 子进程；并发 spawn 的完成顺序由 OS 调度决定，
 * 而 PokePoke 的状态机依赖事件顺序（例如 Pending 必须在 Running 之前到达，否则
 * 「等待你回答」会被后到的 Running 覆盖）。所以这里串成一条链：上一条子进程退出后
 * 才发下一条。
 *
 * 代价可忽略：单条 poke-hook 约 10–20ms，而事件本身稀疏。
 * 加 2s 超时兜底，避免某个子进程卡死把后续通知全部堵住。
 */
let forwardQueue = Promise.resolve()

function forwardOnce(payload) {
  return new Promise((resolve) => {
    let settled = false
    let timer = null
    const done = () => {
      if (settled) return
      settled = true
      if (timer) clearTimeout(timer)
      resolve()
    }
    timer = setTimeout(done, 2000)
    try {
      const child = spawn(POKE_HOOK, ['--source=dsh'], {
        stdio: ['pipe', 'ignore', 'ignore'],
        env: { ...process.env, POKEPOKE_DSH_NAV_SOCKET: SOCKET },
      })
      child.on('error', (error) => {
        debug(`spawn error: ${String(error)}`)
        done()
      })
      child.on('close', done)
      child.stdin.on('error', () => {})
      debug(`forward ${String(payload.hook_event_name)} ${String(payload.session_id)}`)
      child.stdin.write(JSON.stringify(payload))
      child.stdin.end()
    } catch {
      // Never let a notification failure surface into the DSH session.
      done()
    }
  })
}

function forward(payload) {
  forwardQueue = forwardQueue.then(() => forwardOnce(payload))
}

/** Session identity for one event, or null when the session must be ignored. */
function sessionOf(session) {
  const header = session?.header ?? {}
  const id = typeof header.id === 'string' ? header.id : ''
  if (!id) return null
  if (header.origin === 'subagent') return null
  if (typeof header.delegationDepth === 'number' && header.delegationDepth > 0) return null
  return { id, cwd: typeof header.cwd === 'string' ? header.cwd : '' }
}

function send(info, payload) {
  forward({ session_id: info.id, cwd: info.cwd, ...payload })
}

/** Best-effort Session behind a user-questions request (shape is internal API). */
function requestSession(request) {
  return sessionOf(request?.agent?.session ?? request?.session ?? request?.agent)
}

/** Map a DSH LlmFailure onto PokePoke's failure_reason vocabulary. */
function failureReason(error) {
  const raw = String(error?.kind ?? error?.type ?? error?.code ?? '').toLowerCase()
  if (raw.includes('rate') || raw.includes('429')) return 'rate_limit'
  if (raw.includes('overload') || raw.includes('503')) return 'overloaded'
  if (raw.includes('auth') || raw.includes('401') || raw.includes('403')) {
    return 'authentication_failed'
  }
  if (raw.includes('billing') || raw.includes('quota') || raw.includes('payment')) {
    return 'billing_error'
  }
  if (raw.includes('invalid') || raw.includes('400')) return 'invalid_request'
  if (raw.includes('max_output') || raw.includes('max-tokens')) return 'max_output_tokens'
  if (raw.includes('server') || raw.includes('500') || raw.includes('502')) return 'server_error'
  return 'unknown'
}

/** Short human-readable detail for a failed turn. */
function failureDetail(error) {
  for (const key of ['message', 'detail', 'reason', 'code']) {
    const value = error?.[key]
    if (typeof value === 'string' && value.length > 0) return value.slice(0, 300)
  }
  return ''
}

/** session id -> 本次待发的 Stop(等会话树静默后才真正上报)。 */
const pendingStops = new Map()

/** parent session id -> 直接子会话 id 集合,来自 `subagent/catalog` 事件。 */
const childIndex = new Map()

/**
 * 问答的 Pending 有两个来源:agent 级 waterfall `user-questions/request`(按 scope 过滤,
 * 热重载后收不到)和会话日志里的 `tool/call`(name=`ask_user_question`,走未过滤的
 * `session/event`)。两条都接,同一条提问 5s 内只报一次。
 */
const questionSentAt = new Map()

/** callId -> session id:问答工具还在等回答,`tool/result` 到达即恢复 Running。 */
const pendingQuestionCalls = new Map()

const QUESTION_TOOLS = new Set(['ask_user_question'])
const QUESTION_DEDUPE_MS = 5000

/**
 * DSH host 服务,用来判断"这一轮是不是真的结束了"。
 * 惰性绑定:`ctx.inject` 只在服务存在时回调,拿不到就走回落路径。
 */
const services = { agents: undefined, goals: undefined, subagents: undefined }

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

function bindService(ctx, name) {
  try {
    ctx.inject([name], (owner) => {
      try {
        services[name] = owner?.[name]
        debug(`service ${name} bound`)
      } catch {}
    })
  } catch (error) {
    debug(`inject ${name} failed: ${String(error)}`)
  }
}

/** 活着的 Agent。DSH 的 session id 有 `session-` 前缀漂移史,两种写法都试。 */
function liveAgent(sessionId) {
  const agents = services.agents
  if (!agents || typeof agents.get !== 'function') return undefined
  const bare = sessionId.startsWith('session-') ? sessionId.slice(8) : sessionId
  for (const key of [sessionId, `session-${bare}`, bare]) {
    try {
      const agent = agents.get(key)
      if (agent !== undefined) return agent
    } catch {}
  }
  return undefined
}

function hasQueued(agent) {
  const inbox = agent?.inbox
  if (!inbox) return false
  try {
    return (inbox.nextTurn?.length ?? 0) + (inbox.nextStep?.length ?? 0) > 0
  } catch {
    return false
  }
}

/** goal 还在 active+armed ⇒ DSH 自己还要续轮,现在发 Stop 就是假"完成"。 */
function goalKeepsRunning(agent) {
  const goals = services.goals
  if (!goals || typeof goals.get !== 'function') return false
  try {
    const goal = goals.get(agent)
    return goal?.phase === 'active' && goal?.activation === 'armed'
  } catch {
    return false
  }
}

function childRunning(sessionId, seen) {
  const children = childIndex.get(sessionId)
  if (children === undefined) return false
  for (const childId of children) {
    if (seen.has(childId)) continue
    seen.add(childId)
    const agent = liveAgent(childId)
    if (agent !== undefined && agent.status !== 'idle') return true
    if (childRunning(childId, seen)) return true
  }
  return false
}

/**
 * 会话树是否还在自己推进。`''` = 静默(可以上报),其它值是继续等的理由(只进 debug 日志)。
 * Agent 已经不在 host 里时也返回 `''`:没什么可等的,交给 server 侧探活去清。
 */
function busyReason(info) {
  const agent = liveAgent(info.id)
  if (agent === undefined) return ''
  if (agent.status !== 'idle') return 'running'
  if (hasQueued(agent)) return 'queued'
  if (goalKeepsRunning(agent)) return 'goal'
  if (childRunning(info.id, new Set())) return 'child'
  return ''
}

/** 等整棵会话树静默;期间新 turn 开始会通过 entry.cancelled 叫停。 */
async function quietWait(info, entry) {
  const agent = liveAgent(info.id)
  if (agent === undefined) return
  // DSH 自己的 join 原语(goal driver 同款),覆盖 catalog 事件没看见的 external 子会话。
  let join = null
  const subagents = services.subagents
  if (subagents && typeof subagents.waitForChildren === 'function') {
    try {
      join = Promise.resolve(subagents.waitForChildren(agent)).then(
        () => true,
        () => true,
      )
    } catch {}
  }
  const deadline = Date.now() + SETTLE_MAX_MS
  let quietSince = 0
  let lastReason = null
  while (!entry.cancelled) {
    let reason = busyReason(info)
    if (reason === '' && join !== null) {
      const joined = await Promise.race([join, sleep(SETTLE_POLL_MS).then(() => false)])
      if (!joined) reason = 'children'
    }
    if (reason !== lastReason) {
      debug(`settle ${info.id} ${reason === '' ? 'quiet' : reason}`)
      lastReason = reason
    }
    if (reason !== '') quietSince = 0
    else if (quietSince === 0) quietSince = Date.now()
    else if (Date.now() - quietSince >= SETTLE_QUIET_MS) return
    if (Date.now() >= deadline) {
      debug(`settle ${info.id} deadline`)
      return
    }
    await sleep(SETTLE_POLL_MS)
  }
}

async function settleStop(info, payload, entry) {
  if (typeof services.agents?.get === 'function' && liveAgent(info.id) !== undefined) {
    await quietWait(info, entry)
  } else {
    // 服务不可用 / 会话已卸载 → 旧行为(固定去抖)。
    await sleep(STOP_DEBOUNCE_MS)
  }
  if (entry.cancelled) return
  if (pendingStops.get(info.id) !== entry) return
  pendingStops.delete(info.id)
  send(info, payload)
}

function cancelPendingStop(sessionId) {
  const entry = pendingStops.get(sessionId)
  if (entry === undefined) return
  entry.cancelled = true
  pendingStops.delete(sessionId)
  debug(`cancel stop ${sessionId}`)
}

function scheduleStop(info, payload) {
  cancelPendingStop(info.id)
  const entry = { cancelled: false }
  pendingStops.set(info.id, entry)
  debug(`schedule stop ${info.id}`)
  settleStop(info, payload, entry).catch((error) => debug(`settle failed: ${String(error)}`))
}

function handleSessionEvent(session, event) {
  const type = String(event?.type ?? '')
  // catalog 事件父子会话都会收到:先记下 child id,再决定这条会话要不要上报。
  if (type === 'subagent/catalog') {
    const parentId = typeof session?.header?.id === 'string' ? session.header.id : ''
    const childId = typeof event?.data?.childId === 'string' ? event.data.childId : ''
    if (parentId && childId) {
      let children = childIndex.get(parentId)
      if (children === undefined) {
        children = new Set()
        childIndex.set(parentId, children)
      }
      children.add(childId)
      debug(`child ${childId} ${parentId}`)
    }
  }
  const info = sessionOf(session)
  if (!info || !event) return
  switch (type) {
    case 'turn/start':
      // 新 turn 已开始 → 上一次 turn/end 的 Stop 不再代表"等用户"。
      cancelPendingStop(info.id)
      send(info, { hook_event_name: 'UserPromptSubmit' })
      return
    case 'turn/end': {
      const reason = event?.data?.reason ?? {}
      const kind = String(reason.kind ?? '')
      if (kind === 'error') {
        scheduleStop(info, {
          hook_event_name: 'StopFailure',
          reason: failureReason(reason.error),
          detail: failureDetail(reason.error),
        })
      } else if (kind === 'aborted' || kind === 'interrupted') {
        scheduleStop(info, { hook_event_name: 'Stop', aborted: true })
      } else {
        scheduleStop(info, { hook_event_name: 'Stop' })
      }
      return
    }
    case 'tool/call': {
      // 问答工具还没返回 = 会话在等用户回答。走 session/event(未过滤),不依赖
      // agent 级 waterfall,插件热重载后依然有效。
      const name = String(event?.data?.name ?? '')
      if (!QUESTION_TOOLS.has(name)) return
      const callId = String(event?.data?.callId ?? '')
      if (callId) pendingQuestionCalls.set(callId, info.id)
      const questions = questionArgs(event?.data?.arguments)
      questionPending(info, questionText({ questions }))
      return
    }
    case 'tool/result': {
      const callId = String(event?.data?.toolCallId ?? '')
      if (!callId) return
      const waiting = pendingQuestionCalls.get(callId)
      if (waiting === undefined) return
      pendingQuestionCalls.delete(callId)
      send(info, { hook_event_name: 'UserPromptSubmit' })
      return
    }
    case 'approval/asked':
      send(info, {
        hook_event_name: 'PermissionRequest',
        tool_name: String(event?.data?.toolName ?? 'tool'),
      })
      return
    case 'approval/decided': {
      const outcome = event?.data?.outcome ?? {}
      const kind = String(outcome.kind ?? outcome).toLowerCase()
      if (kind.includes('allow') || kind.includes('approve')) {
        send(info, { hook_event_name: 'UserPromptSubmit' })
      }
      return
    }
    default:
      debug(`ignored session event ${type}`)
  }
}

/**
 * Pending for one question, deduped across the two delivery paths.
 * @returns whether this call actually reported (and needed the follow-up Running).
 */
function questionPending(info, message) {
  const last = questionSentAt.get(info.id) ?? 0
  if (Date.now() - last < QUESTION_DEDUPE_MS) return false
  questionSentAt.set(info.id, Date.now())
  send(info, { hook_event_name: 'Notification', message })
  return true
}

/** Parse `tool/call.arguments` (JSON string) into a question list. */
function questionArgs(raw) {
  if (typeof raw !== 'string' || raw.length === 0) return []
  try {
    const args = JSON.parse(raw)
    return Array.isArray(args?.questions) ? args.questions : []
  } catch {
    return []
  }
}

function questionText(request) {
  const list = Array.isArray(request?.questions) ? request.questions : []
  for (const item of list) {
    for (const key of ['question', 'prompt', 'text', 'title']) {
      const value = item?.[key]
      if (typeof value === 'string' && value.trim().length > 0) return value.trim()
    }
  }
  return 'Awaiting your answer'
}

/** Wire shape of the renderer RPC (dsh-client-connection). */
function reply(rpcId, result) {
  return new Response(JSON.stringify({ type: 'server-response', rpcId, result }), {
    headers: { 'content-type': 'application/json' },
  })
}

function parseEnvelope(raw) {
  const envelope = JSON.parse(raw)
  if (envelope?.type !== 'client-request' || typeof envelope.rpcId !== 'string') {
    throw new Error('not-a-client-request')
  }
  return envelope
}

export function apply(ctx) {
  const state = { pending: null, timer: null, showing: null, focused: false, visible: false }

  // 完成判定的状态源;拿不到任何一个是可接受的(回落固定去抖)。
  for (const name of ['agents', 'goals', 'subagents']) bindService(ctx, name)

  const finish = (result) => {
    const pending = state.pending
    state.pending = null
    if (state.timer) {
      clearTimeout(state.timer)
      state.timer = null
    }
    if (!pending) return
    try {
      pending.socket.end(JSON.stringify({ requestId: pending.requestId, ...result }) + '\n')
    } catch {}
    debug(`finish ${JSON.stringify(result)}`)
  }

  try {
    ctx.on('session/event', (session, event) => {
      try {
        handleSessionEvent(session, event)
      } catch (error) {
        debug(`session/event failed: ${String(error)}`)
      }
    })
  } catch (error) {
    debug(`subscribe failed: ${String(error)}`)
  }

  // Questions block the turn until the user answers; PokePoke shows Pending for
  // the wait, then flips back to Running once the answer resolves.
  //
  // 这个 waterfall 只是**兜底**:DSH 用 `ctx.waterfall(scopeTarget(agent, agent), ...)`
  // 派发,是 agent 级 scoped event,cordis 按 listener 的 scope tag 过滤。插件被 profile
  // watcher 热重载后拿到的 ctx 不再是 agent scope 的祖先,连 `{ global: true }` 也收不到
  // (2026-11-10 实测:热重载后 ask_user_question 全程无事件)。真正的 Pending 主路径是
  // `tool/call`(name=ask_user_question)。两条都接,5s 内同一条提问只报一次。
  try {
    ctx.on(
      'user-questions/request',
      (request, next) => {
        let result
        try {
          result = next()
        } catch (error) {
          debug(`user-questions next failed: ${String(error)}`)
          throw error
        }
        try {
          const info = requestSession(request)
          if (info) questionPending(info, questionText(request))
          else debug('question: no session')
        } catch (error) {
          debug(`question forward failed: ${String(error)}`)
        }
        Promise.resolve(result)
          .catch(() => {})
          .then(() => {
            try {
              const info = requestSession(request)
              if (info) send(info, { hook_event_name: 'UserPromptSubmit' })
            } catch (error) {
              debug(`question answered forward failed: ${String(error)}`)
            }
          })
        return result
      },
      { global: true },
    )
  } catch (error) {
    debug(`user-questions subscribe failed: ${String(error)}`)
  }

  ctx.inject(['connection'], (owner) => {
    try {
      const register = (method, handler) =>
        owner.connection.fetch.register({
          path: '/api/pokepokeDsh.' + method,
          methods: ['POST'],
          requestBody: 'buffered',
          fetch: handler,
        })

      register('state', async (request) => {
        let envelope
        try {
          envelope = parseEnvelope(await request.text())
        } catch {
          return new Response(null, { status: 400 })
        }
        const payload = envelope.payload ?? {}
        state.showing = typeof payload.showing === 'string' ? payload.showing : null
        state.focused = payload.focused === true
        state.visible = payload.visible === true
        const value = state.pending
          ? { requestId: state.pending.requestId, sessionId: state.pending.sessionId }
          : null
        return reply(envelope.rpcId, { ok: true, value })
      })

      register('result', async (request) => {
        let envelope
        try {
          envelope = parseEnvelope(await request.text())
        } catch {
          return new Response(null, { status: 400 })
        }
        const payload = envelope.payload ?? {}
        if (state.pending && payload.requestId === state.pending.requestId) {
          finish({
            ok: payload.ok === true,
            showing: typeof payload.showing === 'string' ? payload.showing : null,
            reason: typeof payload.reason === 'string' ? payload.reason : null,
          })
        }
        return reply(envelope.rpcId, { ok: true, value: null })
      })
    } catch (error) {
      debug(`routes failed: ${String(error)}`)
    }
  })

  try {
    if (!existsSync(RUN_DIR)) mkdirSync(RUN_DIR, { recursive: true, mode: 0o700 })
    try {
      unlinkSync(SOCKET)
    } catch {}
    const server = createServer((socket) => {
      let buffer = ''
      socket.setTimeout(OPEN_TIMEOUT_MS + 7000, () => socket.destroy())
      socket.on('error', () => {})
      socket.on('data', (chunk) => {
        buffer += chunk.toString()
        const newline = buffer.indexOf('\n')
        if (newline < 0) return
        let command
        try {
          command = JSON.parse(buffer.slice(0, newline))
        } catch {
          socket.end('{"ok":false,"reason":"invalid-json"}\n')
          return
        }
        socket.removeAllListeners('data')
        if (command?.action === 'state') {
          socket.end(
            JSON.stringify({
              ok: true,
              showing: state.showing,
              focused: state.focused,
              visible: state.visible,
            }) + '\n',
          )
          return
        }
        if (command?.action !== 'open' || typeof command.sessionId !== 'string') {
          socket.end('{"ok":false,"reason":"invalid-command"}\n')
          return
        }
        if (state.pending) {
          socket.end('{"ok":false,"reason":"busy"}\n')
          return
        }
        state.pending = { requestId: String(Date.now()), sessionId: command.sessionId, socket }
        state.timer = setTimeout(() => finish({ ok: false, reason: 'timeout' }), OPEN_TIMEOUT_MS)
        debug(`open request ${command.sessionId}`)
      })
    })
    server.on('error', (error) => debug(`socket error: ${String(error)}`))
    server.listen(SOCKET, () => {
      try {
        chmodSync(SOCKET, 0o600)
      } catch {}
      debug(`socket listening ${SOCKET}`)
    })
    ctx.effect(() => () => {
      try {
        server.close()
      } catch {}
      try {
        unlinkSync(SOCKET)
      } catch {}
    })
  } catch (error) {
    debug(`socket failed: ${String(error)}`)
  }
}
