/**
 * Minimal react-dom/client stub for bundling
 * Browser builds are just bundled assets - no actual rendering happens in the bundler
 */
function createRoot() {
  return {
    render: function() {},
    unmount: function() {}
  };
}

function hydrateRoot() {
  return {
    render: function() {},
    unmount: function() {}
  };
}

export { createRoot, hydrateRoot };
export default { createRoot, hydrateRoot };