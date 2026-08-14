const encoder = new TextEncoder();
const decoder = new TextDecoder();

function requireDsFilename(filename) {
  if (typeof filename !== "string" || !filename.endsWith(".ds")) {
    throw new TypeError("Deka demo inputs must be named .ds files");
  }
}

function bytesToDataUrl(bytes) {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return `data:text/javascript;base64,${btoa(binary)}`;
}

function resultFromAbi(memory, resultPtr) {
  const header = new DataView(memory.buffer, resultPtr, 8);
  const jsonPtr = header.getUint32(0, true);
  const jsonLen = header.getUint32(4, true);
  const response = JSON.parse(decoder.decode(new Uint8Array(memory.buffer, jsonPtr, jsonLen)));
  return { response, allocationSize: 8 + jsonLen };
}

function createRuntime(results, consoleOutput) {
  const console = Object.freeze({
    log: (...values) => consoleOutput.push(values.map(String).join(" ")),
    info: (...values) => consoleOutput.push(values.map(String).join(" ")),
    warn: (...values) => consoleOutput.push(values.map(String).join(" ")),
    error: (...values) => consoleOutput.push(values.map(String).join(" ")),
  });
  return Object.freeze({
    console,
    result: (value) => results.push(value),
  });
}

export function renderDekaDemoOutcome(element, outcome) {
  if (!element) throw new TypeError("an outcome element is required");
  element.dataset.dekaOutcome = outcome.kind;
  if (outcome.kind === "success") {
    element.textContent = outcome.results.map(String).join("\n");
  } else if (outcome.kind === "diagnostic") {
    element.textContent = outcome.diagnostics
      .map((diagnostic) => `${diagnostic.filename}:${diagnostic.start_line}:${diagnostic.start_column} ${diagnostic.message}`)
      .join("\n");
  } else if (outcome.kind === "runtime_error") {
    element.textContent = `Runtime error: ${outcome.error}`;
  } else {
    throw new TypeError(`unknown Deka demo outcome: ${outcome.kind}`);
  }
}

/**
 * Consumer for the v1 Deka compiler browser ABI. It intentionally accepts one
 * named DekaScript input rather than a project bundle or alternate language.
 */
export function createDekaDemoConsumer({ wasmUrl, fetchImpl = globalThis.fetch, instantiate = WebAssembly.instantiate, importModule = (url) => import(url) }) {
  if (!wasmUrl) throw new TypeError("wasmUrl is required");
  let compilerPromise;
  let execution = Promise.resolve();

  const compiler = () => {
    compilerPromise ??= (async () => {
      const response = await fetchImpl(wasmUrl);
      if (!response.ok) throw new Error(`unable to load Deka compiler WASM: ${response.status}`);
      const { instance } = await instantiate(await response.arrayBuffer());
      const exports = instance.exports;
      for (const name of ["memory", "deka_compiler_alloc", "deka_compiler_compile", "deka_compiler_free"]) {
        if (!(name in exports)) throw new Error(`Deka compiler ABI is missing ${name}`);
      }
      return exports;
    })();
    return compilerPromise;
  };

  const compile = async ({ filename, source }) => {
    requireDsFilename(filename);
    if (typeof source !== "string") throw new TypeError("Deka demo source must be text");
    const e = await compiler();
    const allocations = [source, filename, "deka"].map((value) => {
      const bytes = encoder.encode(value);
      const ptr = e.deka_compiler_alloc(bytes.length);
      new Uint8Array(e.memory.buffer, ptr, bytes.length).set(bytes);
      return [ptr, bytes.length];
    });
    let resultPtr;
    let resultSize = 0;
    try {
      resultPtr = e.deka_compiler_compile(...allocations.flat());
      const decoded = resultFromAbi(e.memory, resultPtr);
      resultSize = decoded.allocationSize;
      return decoded.response;
    } finally {
      for (const [ptr, size] of allocations) e.deka_compiler_free(ptr, size);
      if (resultPtr) e.deka_compiler_free(resultPtr, resultSize);
    }
  };

  const run = async (input) => {
    let release;
    const previous = execution;
    execution = new Promise((resolve) => { release = resolve; });
    await previous;
    try {
      const compiled = await compile(input);
      if (!compiled.ok) return { kind: "diagnostic", diagnostics: compiled.diagnostics, metadata: compiled.metadata };

      const results = [];
      const consoleOutput = [];
      const runtime = createRuntime(results, consoleOutput);
      const previousRuntime = globalThis.runtime;
      const previousConsole = globalThis.console;
      globalThis.runtime = runtime;
      globalThis.console = runtime.console;
      try {
        await importModule(bytesToDataUrl(encoder.encode(compiled.output.code)));
        return { kind: "success", results, console: consoleOutput, metadata: compiled.metadata };
      } catch (error) {
        return { kind: "runtime_error", error: String(error?.message ?? error), metadata: compiled.metadata };
      } finally {
        if (previousRuntime === undefined) delete globalThis.runtime;
        else globalThis.runtime = previousRuntime;
        globalThis.console = previousConsole;
      }
    } finally {
      release();
    }
  };

  return Object.freeze({ compile, run });
}
