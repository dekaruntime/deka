const Color = Object.freeze({ Red: 0, Green: 1 });
let sink = 0;
for (let i = 0; i < 2000000; i += 1) {
  sink += 1;
}
console.log(sink);
