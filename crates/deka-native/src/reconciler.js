// Host adapter for React 19.1.1 / react-reconciler 0.32.0. No DOM objects.
import React from 'deka:///js/react.js';
import * as Scheduler from 'deka:///js/scheduler.js';
const module = {exports: {}};
const require = id => {
  if (id === 'react') return React;
  if (id === 'scheduler') return Scheduler;
  throw new Error(`Unsupported reconciler dependency: ${id}`);
};
/*__RECONCILER__*/
const createReconciler = module.exports;
let priority = 32;
let nextId = 1;
let version = 0;
let failure = null;
const container = {children: []};
const report = error => { failure = String(error && error.stack || error); };
const remove = (parent, child) => {
  const index = parent.children.indexOf(child);
  if (index >= 0) parent.children.splice(index, 1);
};
const append = (parent, child) => { remove(parent, child); parent.children.push(child); };
const insert = (parent, child, before) => {
  remove(parent, child);
  const index = parent.children.indexOf(before);
  if (index < 0) throw new Error('Native insertion target is missing');
  parent.children.splice(index, 0, child);
};
const validate = (type, props) => {
  if (!['div', 'span', 'p', 'button'].includes(type)) throw new Error(`Unsupported native element: ${type}`);
  for (const name of Object.keys(props)) {
    if (!['children', 'className', 'onClick', 'id', 'ref', 'data-deka-id'].includes(name)) throw new Error(`Unsupported native prop: ${name}`);
  }
  if (props.className != null && typeof props.className !== 'string') throw new Error('Native className must be a string');
  if (props.onClick != null && typeof props.onClick !== 'function') throw new Error('Native onClick must be a function');
};
const renderer = createReconciler({
  rendererVersion: '0.1.0', rendererPackageName: 'deka-native',
  isPrimaryRenderer: true, supportsMutation: true, supportsPersistence: false,
  supportsHydration: false, supportsResources: false, supportsSingletons: false,
  getRootHostContext: () => null, getChildHostContext: () => null,
  getPublicInstance: instance => Object.freeze({id: String(instance.id)}),
  prepareForCommit: () => null, resetAfterCommit: () => { version++; },
  createInstance(type, props) {
    validate(type, props);
    return {id: nextId++, type, props, children: [], hidden: false};
  },
  createTextInstance: text => ({id: nextId++, text, hidden: false}),
  appendInitialChild: append, appendChild: append, appendChildToContainer: append,
  insertBefore: insert, insertInContainerBefore: insert,
  removeChild: remove, removeChildFromContainer: remove,
  finalizeInitialChildren: () => false, shouldSetTextContent: () => false,
  commitUpdate(instance, type, oldProps, props) { validate(type, props); instance.props = props; },
  commitTextUpdate(instance, oldText, text) { instance.text = text; },
  clearContainer: target => { target.children = []; },
  hideInstance: instance => { instance.hidden = true; },
  hideTextInstance: instance => { instance.hidden = true; },
  unhideInstance: instance => { instance.hidden = false; },
  unhideTextInstance: instance => { instance.hidden = false; },
  resetTextContent: instance => { instance.children = []; },
  detachDeletedInstance: () => {}, preparePortalMount: () => { throw new Error('Native portals are unsupported'); },
  scheduleTimeout: setTimeout, cancelTimeout: clearTimeout, noTimeout: -1,
  supportsMicrotasks: true, scheduleMicrotask: queueMicrotask,
  setCurrentUpdatePriority: value => { priority = value; },
  getCurrentUpdatePriority: () => priority, resolveUpdatePriority: () => priority || 32,
  shouldAttemptEagerTransition: () => false, maySuspendCommit: () => false,
});
const root = renderer.createContainer(container, 1, null, false, null, 'native-', report, report, report, null);
const handlers = new Map();
function snapshot(instance) {
  if (instance.hidden) return null;
  if ('text' in instance) return {id: String(instance.id), text: instance.text, children: []};
  const props = instance.props || {};
  let handler = null;
  if (props.onClick) { handler = instance.id; handlers.set(handler, props.onClick); }
  return {id: String(instance.id || 'root'), tag: instance.type || 'div', classes: props.className || '',
    handler, children: instance.children.map(snapshot).filter(Boolean)};
}
function flush() {
  renderer.flushSyncWork();
  renderer.flushPassiveEffects();
  renderer.flushSyncWork();
  if (failure !== null) { const error = failure; failure = null; throw new Error(error); }
}
globalThis.__dekaNative = Object.freeze({
  mount(Component) {
    if (typeof Component !== 'function') throw new Error('Native entry must export a component function');
    renderer.updateContainerSync(React.createElement(Component), root, null, null);
    flush();
  },
  frame() {
    flush();
    handlers.clear();
    const tree = snapshot(container);
    return {version, tree};
  },
  click(id) {
    const handler = handlers.get(id);
    if (!handler) throw new Error(`Native handler ${id} is no longer mounted`);
    renderer.flushSyncFromReconciler(() => {
      const result = handler();
      if (result && typeof result.then === 'function') result.catch(report);
    });
    flush();
  },
  unmount() { renderer.updateContainerSync(null, root, null, null); flush(); handlers.clear(); },
});
