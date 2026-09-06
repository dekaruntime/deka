let sink = 0;
for (let i = 0; i < 2000000; i += 1) {
  const c = i;
  sink += c;
}
console.log(sink);
