import { readFile } from "node:fs/promises";

const artifact = process.argv[2];
if (!artifact) throw new Error("usage: bun browser-parity.mjs <compiler.wasm>");
const { instance } = await WebAssembly.instantiate(await readFile(artifact));
const e = instance.exports;

for (const name of ["phpx_compile", "phpx_compile_alloc", "phpx_compile_free"]) {
  if (name in e) throw new Error(`removed PHPX ABI export is still public: ${name}`);
}
for (const name of ["deka_compiler_alloc", "deka_compiler_compile", "deka_compiler_free", "deka_compiler_metadata"]) {
  if (!(name in e)) throw new Error(`required Deka ABI export is missing: ${name}`);
}

function write(value) {
  const bytes = new TextEncoder().encode(value);
  const ptr = e.deka_compiler_alloc(bytes.length);
  new Uint8Array(e.memory.buffer, ptr, bytes.length).set(bytes);
  return [ptr, bytes.length];
}

function compile(source, filename, mode) {
  const [sourcePtr, sourceLen] = write(source);
  const [filenamePtr, filenameLen] = write(filename);
  const [modePtr, modeLen] = write(mode);
  const resultPtr = e.deka_compiler_compile(sourcePtr, sourceLen, filenamePtr, filenameLen, modePtr, modeLen);
  const header = new DataView(e.memory.buffer, resultPtr, 8);
  const jsonPtr = header.getUint32(0, true);
  const jsonLen = header.getUint32(4, true);
  const response = JSON.parse(new TextDecoder().decode(new Uint8Array(e.memory.buffer, jsonPtr, jsonLen)));
  e.deka_compiler_free(sourcePtr, sourceLen);
  e.deka_compiler_free(filenamePtr, filenameLen);
  e.deka_compiler_free(modePtr, modeLen);
  e.deka_compiler_free(resultPtr, 8 + jsonLen);
  return response;
}

const success = compile("const answer = 42;", "lesson.ds", "auto");
if (!success.ok || success.metadata.language !== "deka" || !success.output?.code?.includes("const answer = deka.freeze(42)")) {
  throw new Error(`successful .ds compile did not match the ABI contract: ${JSON.stringify(success)}`);
}

const rejectedFilename = compile("function greeting($name: string): string { return $name; }", "legacy.phpx", "phpx");
if (rejectedFilename.ok || !rejectedFilename.diagnostics?.[0]?.message.includes("only accepts .ds")) {
  throw new Error(`PHPX filename fallback was not rejected: ${JSON.stringify(rejectedFilename)}`);
}

const rejectedMode = compile("const answer = 42;", "lesson.ds", "phpx");
if (rejectedMode.ok || !rejectedMode.diagnostics?.[0]?.message.includes("supported modes are `auto` and `deka`")) {
  throw new Error(`PHPX mode fallback was not rejected: ${JSON.stringify(rejectedMode)}`);
}

const failure = compile("function broken(", "broken.ds", "deka");
const diagnostic = failure.diagnostics?.[0];
if (failure.ok || diagnostic?.severity !== "error" || diagnostic.filename !== "broken.ds" || !Number.isInteger(diagnostic.start_line)) {
  throw new Error(`diagnostic compile did not match the ABI contract: ${JSON.stringify(failure)}`);
}

const tourCases = JSON.parse(await readFile(new URL("./fixtures/deka-tour-sources.json", import.meta.url)));
if (tourCases.length !== 36) {
  throw new Error(`expected 36 website tour sources, found ${tourCases.length}`);
}
for (const testCase of tourCases) {
  const response = compile(testCase.source, "tour.ds", "deka");
  if (response.ok !== testCase.expect_compile) {
    throw new Error(`${testCase.name} browser WASM compile result drifted: ${JSON.stringify(response)}`);
  }
  if (testCase.expect_compile && typeof response.output?.code !== "string") {
    throw new Error(`${testCase.name} did not return browser WASM output: ${JSON.stringify(response)}`);
  }
  if (!testCase.expect_compile && !response.diagnostics?.some((diagnostic) => diagnostic.message?.includes(testCase.expect_error))) {
    throw new Error(`${testCase.name} browser WASM diagnostic drifted: ${JSON.stringify(response)}`);
  }
}

const structSource = `struct Point {
  x: number
  y: number
}

const origin = Point { x: 3, y: 4 };
console.log(origin.x + origin.y);`;
const structResponse = compile(structSource, "structs.ds", "deka");
if (!structResponse.ok || !structResponse.output?.code) {
  throw new Error(`struct compile failed: ${JSON.stringify(structResponse)}`);
}
// The emitted JS is executed by the tour in a strict-mode function. Ensure the
// deka.Struct factory does not assign to f.name (which is non-writable in strict
// mode and throws "Attempted to assign to readonly property.").
const structCode = structResponse.output.code
  .replace(/^export const \w+ = [^;]+;\n?/gm, "")
  .replace(/^export async function \w+[\s\S]*$/m, "");
const stdout = [];
try {
  const run = new Function(
    "console",
    "stdout",
    `"use strict";\n${structCode}\nreturn stdout;`,
  );
  run({ log: (...args) => stdout.push(args.join(" ")) }, stdout);
} catch (error) {
  throw new Error(`strict-mode struct execution failed: ${error.message}\n${structCode}`);
}
if (!stdout.some((line) => line.includes("7"))) {
  throw new Error(`expected struct output to contain 7, got: ${JSON.stringify(stdout)}`);
}

console.log("browser WASM parity fixtures passed");
