const type = { kind: "string", name: "string", toString() { return this.name; } };
let sink = 0;
for (let i = 0; i < 2000000; i += 1) {
  if (type.toString() === "string") sink += 1;
}
console.log(sink);
