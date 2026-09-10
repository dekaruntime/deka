const __dekaBuildHydrate = (factories, descriptor, value, strict) => {
switch (descriptor.node) {
case "leaf":
return descriptor.kind === "bytes" ? new Uint8Array(value.__deka_bytes) : value;
case "array": return value.map((item) => __dekaBuildHydrate(factories, descriptor.elem, item, strict));
case "struct": {
const fields = {};
for (const field of descriptor.fields) if (Object.hasOwn(value, field.name)) fields[field.name] = __dekaBuildHydrate(factories, field.ty, value[field.name], strict);
const factory = factories[descriptor.name];
if (typeof factory === "function") return factory(fields);
if (strict) throw new Error(`build hydration is missing the declared ${descriptor.name} factory`);
const proto = Object.create(null);
Object.defineProperty(proto, "__deka_struct", { value: descriptor.name, enumerable: false });
const out = Object.create(proto);
Object.assign(out, fields);
return out;
}
case "interface": return value;
case "newtype": {
const payload = __dekaBuildHydrate(factories, descriptor.repr, value, strict);
const factory = factories[descriptor.name];
if (typeof factory === "function") return factory(payload);
if (strict) throw new Error(`build hydration is missing the declared ${descriptor.name} factory`);
const proto = Object.create(null);
Object.defineProperty(proto, "__deka_newtype", { value: descriptor.name, enumerable: false });
Object.defineProperty(proto, Symbol.for("deka.nt"), { value: payload, enumerable: false });
proto.toJSON = function () { return this[Symbol.for("deka.nt")]; };
return Object.create(proto);
}
case "enum": {
const item = descriptor.cases.find(([name]) => name === value.__case);
const factory = factories[descriptor.name];
if (factory) return item[1] === null
? factory[value.__case]
: factory[value.__case](__dekaBuildHydrate(factories, item[1], value.value, strict));
if (strict) throw new Error(`build hydration is missing the declared ${descriptor.name} factory`);
return item[1] === null
? { __enum: descriptor.name, __case: value.__case, name: value.__case }
: { __enum: descriptor.name, __case: value.__case, name: value.__case, value: __dekaBuildHydrate(factories, item[1], value.value, strict) };
}
case "option": return value.__case === "None"
? { __enum: "Option", __case: "None", name: "None" }
: { __enum: "Option", __case: "Some", name: "Some", value: __dekaBuildHydrate(factories, descriptor.inner, value.value, strict) };
case "union": {
for (const member of descriptor.members) {
try { return __dekaBuildHydrate(factories, member, value, strict); } catch (_err) {}
}
throw new Error("build value does not match a union member");
}
default: throw new Error(`unsupported build descriptor ${descriptor.node}`);
}
};
const __dekaBuildDescriptor = {"elem":{"fields":[{"name":"slug","optional":false,"ty":{"kind":"string","name":"string","node":"leaf"}}],"name":"PostParam","node":"struct"},"node":"array"};
const __dekaBuildValue = [{"slug":"hello"},{"slug":"world"}];
export const hydrate = (factories = {}) => __dekaBuildHydrate(factories, __dekaBuildDescriptor, __dekaBuildValue, true);
export const value = __dekaBuildHydrate({}, __dekaBuildDescriptor, __dekaBuildValue, false);
