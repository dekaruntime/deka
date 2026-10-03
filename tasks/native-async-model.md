# APS 7: native async execution

> **State: committed model, revised 3 October 2026.** This replaces APS 7's V8/JavaScript scheduling and promise-rejection rules with the decisions in the native port's note 07. It specifies the target contract; recording the model does not claim every implementation task has shipped. Explicit Promise signatures and the distinction between Result data and Throw/Exception remain.

## Execution and ownership

One VM is driven as one Rust future on tokio. The VM owns DekaScript tasks, promise values, suspended frames and their garbage-collection roots. Tokio drives the VM and pending Rust operations; the VM schedules language work. JavaScript's event loop and microtask queue are not part of this contract.

A CLI program uses a single-threaded tokio runtime with timers and I/O enabled. Multiple operations can be pending concurrently without running DekaScript code concurrently. Native operations may use Rust's own concurrency internally, but callbacks enter the VM through its scheduler rather than executing language code on arbitrary host threads.

Calling an async function creates a VM task and returns its Promise. The caller can retain that Promise and begin other work before awaiting it. Async signatures remain explicit: an async function returning a number declares `Promise<number>`. An async function returning a Result declares `Promise<Result<T, E>>`; async does not silently rewrite the authored return type.

## Await and completion

`await` inspects a Promise. A completed Promise supplies its value; an unfinished Promise parks only the current task, preserving its instruction position, frames and rooted values. Other ready tasks keep running. When a producer or host future settles, the consumer can become ready and resume. Nested awaits and awaits inside loops follow the same mechanism.

The main task's normal return does not cancel work. The CLI preserves its result and continues driving every running task, including tasks created while draining existing work. It exits successfully only when that work is complete. A pending task keeps the process alive even when no task is currently ready; completion must not be inferred from one idle poll. Explicit cancellation and abnormal shutdown release the affected tasks' pending Rust futures.

A desktop runtime remains alive until its window closes. Returning from initialization or an event handler does not dispose of that runtime or discard its pending work. When async work completes and changes state, the view can update through the component/binding system.

## Failures across await

Operational host failures are values: a fallible operation resolves to `Result<T, E>`, with `Ok(value)` or `Err(error)`. A failed read or request is not converted into a Throw at an await site. `try` handles authored Throw/Exception control flow; it is not the I/O error-handling mechanism.

Await does not erase an authored Exception return type or merge it with Result. The same checked handling rules used for synchronous Throw/Exception remain in force for language code. Invalid bytecode, incorrect host wire types and VM invariant failures remain runtime faults rather than being disguised as operational Err values. Uncaught Throw and runtime faults cause nonzero CLI exit with a diagnostic.

The Result constructor, checker signature and VM settlement path are shared with synchronous Result values. There is no JavaScript rejection channel, summon membrane or `unsafe { await ... }` conversion in the native model.

## Window integration and fairness

The VM exposes a backend-independent interface to run one turn within an instruction budget, report whether work is ready, and register/notify a wake. Host futures use the supplied wake mechanism. Window backends call this interface; they do not inspect task internals or implement a second language scheduler.

The window layer in `deka_native_ui` is migrating from GPUI to winit/wgpu/vello. Both integrations call the same VM interface during that migration. Renderer and text implementation details stay outside the VM; backend glue schedules turns and requests view updates.

A turn has a finite instruction budget across the VM's work. A task that never awaits cannot block a window indefinitely, and a VM with ready work yields control when its turn is spent. The budget limits a turn rather than imposing a lifetime instruction quota on long-lived applications. Ready work schedules another turn; an idle VM waits for an actual wake instead of spinning.

## APIs built on this model

The runtime provides `time.sleep(ms)` and the familiar timer globals `setTimeout`, `setInterval`, `clearTimeout` and `clearInterval`. Timer callbacks enter the same task scheduler. Clearing a timer removes its pending callback work; timers and other host operations do not require a JavaScript runtime.

`Promise.all` and `Promise.race` retain their familiar names and consume Promise values. Waiting on many operations, cancelling one task and dropping its Rust future are VM/runtime facilities. Detailed API types and settlement behavior belong to their implementation tasks rather than being inferred from JavaScript compatibility.

Records, lists, bytes and opaque host-owned handles can cross the host boundary. Host handles allow resources such as response bodies and sockets to stay owned by Rust. A host operation can arrange for a DekaScript callback, but it does so through the VM's turn/wake interface and retains the callback's values safely until delivery or cancellation.

## Scope and validation

The native port note 07 is the implementation order. A successful implementation must prove CLI task draining, real asynchronous host work, deterministic timers, richer host values, async desktop handlers, Promise combinators, task cancellation, bounded turns and the async corpus.

Its acceptance test is fetch from both the CLI and a desktop button: concurrent requests, an operational failure represented by Err, cancellation of one request, and a responsive window throughout. Async tests drive the scheduler to completion or advance the VM/executor clock; they do not sleep in hopes that another thread has finished.

This model supersedes the previous V8-specific scheduling, rejection-as-throw, JavaScript erasure, summoned async defaults and no-cancellation policy in APS 7. It does not reopen the settled decision to retain try and Exception or to use Result for I/O.

-codex
