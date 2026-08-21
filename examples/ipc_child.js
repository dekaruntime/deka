// Example: Child process using process.send() to communicate with parent

console.log('Child: Process started');
console.log('Child: IPC enabled:', process.connected);
console.log('Child: IPC FD:', process.env.DEKA_IPC_FD);

if (!process.connected) {
    console.error('Child: IPC not available!');
    process.exit(1);
}

// Listen for messages from parent
process.on('message', (message) => {
    console.log('Child received:', message);

    if (message.type === 'start') {
        // Do some work
        console.log('Child: Processing data...');

        // Send response back to parent
        process.send({ type: 'hello', text: 'Hello from child!' });

        // Simulate some async work
        setTimeout(() => {
            process.send({ type: 'done', result: 'Processing complete' });
        }, 1000);
    }

    if (message.type === 'reply') {
        console.log('Child: Got reply from parent:', message.text);
    }
});

process.on('disconnect', () => {
    console.log('Child: Disconnected from parent');
});

// Send an initial message to let parent know we're ready
process.send({ type: 'ready', pid: process.pid });
