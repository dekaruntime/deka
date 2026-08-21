// Example: Parent process spawning a child with IPC enabled
const { ChildProcess } = await import('deka/child_process');

console.log('Parent: Starting child process with IPC...');

const child = new ChildProcess('deka', ['run', 'examples/ipc_child.js'], {
    stdio: 'pipe',
    ipc: true
});

// Listen for messages from child
child.on('message', (message) => {
    console.log('Parent received:', message);

    // Reply back to child
    if (message.type === 'hello') {
        child.send({ type: 'reply', text: 'Hello from parent!' });
    }

    if (message.type === 'done') {
        console.log('Parent: Child finished, disconnecting...');
        child.disconnect();
    }
});

child.on('disconnect', () => {
    console.log('Parent: IPC channel closed');
});

child.on('exit', (code) => {
    console.log(`Parent: Child exited with code ${code}`);
});

// Send initial message to child
child.send({ type: 'start', data: 'Begin processing' });
