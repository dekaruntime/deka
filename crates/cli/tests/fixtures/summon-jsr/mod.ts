import { add, type Pair } from "./lib/add.ts";
export interface Answer { value: number }
export function answer(): number {
  const pair: Pair = { left: 20, right: 22 };
  return add(pair);
}
export { Kind } from "./lib/add.ts";
export async function later(): Promise<number> {
  const { add } = await import("./lib/add.ts");
  return add({ left: 1, right: 2 });
}
