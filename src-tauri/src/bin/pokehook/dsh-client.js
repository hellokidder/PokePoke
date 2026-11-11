// PokePoke DeepSeek Harness integration — renderer half.
// pokepoke-plugin-marker
// Do not edit by hand; re-run Connect from the PokePoke tray to regenerate.
//
// DSH boots this bundle through its client module system. It polls the host half
// for an "open conversation" request and performs it in the renderer, which is
// the only place `uiWorkspace.openSession` exists.
window.__ModuleLoader__.load({
  id: 'pokepoke-dsh',
  factory() {
    const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
    const frames = () =>
      new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)))

    return {
      name: 'pokepoke-dsh',
      apply(ctx) {
        ctx.inject(['connection', 'uiWorkspace', 'sessions'], (owner) => {
          const life = new AbortController()
          let busy = false

          const showing = () =>
            Object.values(owner.sessions.list.getSnapshot().byId).find(
              (row) => ((row.retainedBy && row.retainedBy.mainView) || 0) > 0,
            )?.id ?? null
          const focused = () =>
            document.visibilityState === 'visible' && document.hasFocus()
          const rpc = (method, payload) =>
            owner.connection.rpc.call('/api', 'pokepokeDsh.' + method, payload, life.signal)

          const openSession = async (id) => {
            if (!owner.sessions.list.getSnapshot().byId[id]) return 'missing-session'
            // PokePoke activates the app right before issuing the request; give
            // the window a moment to actually become frontmost.
            let waited = 0
            while (!focused() && waited < 1500) {
              await sleep(100)
              waited += 100
            }
            if (!focused()) return 'viewer-not-focused'

            if (typeof owner.sessions.using === 'function') {
              try {
                const reason = await owner.sessions.using(
                  id,
                  { source: 'controllerOperation', signal: life.signal },
                  async (ref) => {
                    await ref.ready
                    if (!focused()) return 'viewer-not-focused'
                    owner.uiWorkspace.openSession(id)
                    await frames()
                    return showing() === id ? null : 'not-switched'
                  },
                )
                if (reason === null && showing() === id) return null
                if (reason !== null) return reason
              } catch {
                // Fall through to the plain navigation below.
              }
            }

            owner.uiWorkspace.openSession(id)
            for (let waited2 = 0; waited2 < 4000; waited2 += 100) {
              await sleep(100)
              if (showing() === id) return null
            }
            return 'not-switched'
          }

          const tick = async () => {
            try {
              const reply = await rpc('state', {
                showing: showing(),
                focused: focused(),
                visible: document.visibilityState === 'visible',
              })
              if (!reply || !reply.ok) return
              const command = reply.value
              if (!command || !command.sessionId || busy) return
              busy = true
              let result
              try {
                const reason = await openSession(command.sessionId)
                result = { ok: reason === null, showing: showing(), reason }
              } catch (error) {
                result = { ok: false, showing: showing(), reason: String(error) }
              }
              await rpc('result', { requestId: command.requestId, ...result })
              busy = false
            } catch {
              // Host not reachable yet (reload, HMR): retry on the next tick.
            }
          }

          void tick()
          const timer = setInterval(tick, 500)
          // 焦点变化立刻上报,别等下一次轮询 —— PokePoke 的 popup gate 会
          // 在 turn/end 时通过 host 半 socket 现查这些状态。
          const onFocusChange = () => {
            void tick()
          }
          window.addEventListener('focus', onFocusChange)
          window.addEventListener('blur', onFocusChange)
          document.addEventListener('visibilitychange', onFocusChange)
          owner.effect(() => () => {
            clearInterval(timer)
            life.abort()
            window.removeEventListener('focus', onFocusChange)
            window.removeEventListener('blur', onFocusChange)
            document.removeEventListener('visibilitychange', onFocusChange)
          })
        })
      },
    }
  },
})
