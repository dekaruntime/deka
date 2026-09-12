export type Pair = { left: number; right: number };
export enum Kind { Answer = 42 }
export function add(pair: Pair): number { return pair.left + pair.right; }
