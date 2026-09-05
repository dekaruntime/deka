const c = { value: 0, inc() { this.value += 1; } };
for (let i = 0; i < 2000000; i += 1) c.inc();
console.log(c.value);
