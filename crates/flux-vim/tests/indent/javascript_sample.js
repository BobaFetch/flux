// A sample for comparing highlights.
import { readFile } from "fs/promises";

const MAX = 10;
let count = 0;

/** A documented class. */
class Counter extends Object {
  #secret = 42;
  constructor(start = 0) {
    super();
    this.value = start;
  }
  async load(path) {
    const text = await readFile(path, "utf8");
    return text.split(/\n/).length;
  }
  static of(n) { return new Counter(n); }
}

export function main(args) {
  for (const a of args) {
    if (a === null || typeof a !== "string") continue;
    console.log(`arg: ${a} ${count++}`, true, undefined);
  }
  const el = <div className="x">{MAX}</div>;
  return args.map((x) => x * 2);
}
