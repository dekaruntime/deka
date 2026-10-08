// Only linked by the web-test feature, never by the shipped tour.
import { mount as mountFixture, unmount, reportPanic, wake } from './host.js'
export { unmount, reportPanic, wake }
export function mount(app, canvas) {
  const frame = app.frame_at.bind(app)
  app.frame_at = (...args) => {
    const start = performance.now()
    const result = frame(...args)
    ;(window.dekaFrameTimes ??= []).push({ms:performance.now()-start,bytes:result.length})
    return result
  }
  return mountFixture(app, canvas, true)
}
