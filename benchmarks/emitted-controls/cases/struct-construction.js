let sink = 0;
for (let i = 0; i < 2000000; i += 1) {
  const p = { x: i, y: i + 1 };
  sink += p.y;
}
console.log(sink);
