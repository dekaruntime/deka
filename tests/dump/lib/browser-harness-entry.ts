import {
  runDekaJs,
  setCompilerArtifactPath,
  terminateSharedSandbox,
} from '@dekaruntime/web-ide-kit/runtime'

setCompilerArtifactPath('https://wasm.deka.gg/latest/deka-compiler-artifact.json')

const g = globalThis as typeof globalThis & {
  __dekaRunJs?: typeof runDekaJs
  __dekaTerminate?: typeof terminateSharedSandbox
}

g.__dekaRunJs = runDekaJs
g.__dekaTerminate = terminateSharedSandbox
