let sink = 0;
for (let i = 0; i < 2000000; i += 1) {
  const props = { class: "card", role: "listitem" };
  sink += props.class.length + props.role.length;
}
console.log(sink);
