let sink = 0;
const values = [1, 2, 3];
for (let i = 0; i < 2000000; i += 1) {
  sink += "abcdef".slice(1, 5).length + values.length;
}
console.log(sink);
