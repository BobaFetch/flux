/** A documented counter. */
class Counter extends Base {
  #secret = 42;
  static count = 0;

  constructor(start = 0) {
    super();
    this.value = start;
  }

  // Load a file.
  async load(file) {
    const text = await readFile(file, "utf8");
    return text.split("\n").length;
  }

  get double() {
    return this.value * 2;
  }

  *items() {
    yield 1;
    yield 2;
  }
}

export class Other {
  method() {
    if (this.ok) {
      return new Counter(1);
    }
    return null;
  }
}

/* a block comment */
export const instance = new Counter(3);
