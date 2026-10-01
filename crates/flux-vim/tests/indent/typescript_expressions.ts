const ids: Array<Id> = [1, "two"];

const config = {
  name: "flux",
  nested: {
    list: [1, 2, 3],
  },
};

const total = values
  .map((v) => v * 2)
  .filter((v) => v > 2);

const sum = 1 +
  2 +
  3;

function run(kind: string): number {
  switch (kind) {
    case "a":
      return 1;
    default:
      return 0;
  }
}

if (ready)
  start();
else
  stop();

for (const id of ids)
  console.log(id);

while (pending > 0)
  pending--;

call(first,
  second);

const handler = (event: Event): void => {
  event.preventDefault();
};

promise.then((value) => {
  console.log(value);
});

const label = `id: ${ids[0]}`;
const pattern = /ab+c/g;
let c = new Circle(2) as Shape;
