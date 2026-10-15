// Imported only by debug WASM with the hot-reload feature.
import { mount } from './host.js'

export function mountHot(app, canvas) {
  const events = new EventSource('/__deka/events')
  let disposed = false
  const restartMessage = reason => `deka dev: ${reason}${reason.includes('signal state resets') ? '' : '; rebuilding and restarting (signal state resets)'}`
  let restarting = false
  const report = async result => {
    if (result.restart && !restarting) {
      restarting = true
      console.warn(restartMessage(result.restart))
      await fetch('/__deka/rebuild', { method: 'POST' })
    } else if (result.error) {
      console.warn(`deka dev: ${result.error}; keeping last good UI`)
    } else if (result.patched) {
      console.info(`deka dev: patched ${result.patched} template(s); state preserved`)
    }
  }
  const frame = app.frame_at.bind(app)
  app.frame_at = (...args) => {
    const value = frame(...args)
    const status = JSON.parse(value).hot_reload
    if (status) report(status).catch(error => console.error('deka dev: reload failed:', error))
    return value
  }
  const handle = mount(app, canvas)
  events.onmessage = async event => {
    if (disposed) return
    try {
      const message = JSON.parse(event.data)
      if (message.type === 'reload') { location.reload(); return }
      if (message.type === 'restart') {
        console.warn(restartMessage(message.reason))
        return
      }
      if (message.type === 'error') {
        console.warn(`deka dev: ${message.reason}; keeping last good UI`)
        return
      }
      const result = JSON.parse(app.hot_reload(event.data))
      await report(result)
    } catch (error) {
      console.error('deka dev: browser hot reload failed:', error)
    }
  }
  return { dispose() { disposed = true; events.close(); handle.dispose() } }
}
