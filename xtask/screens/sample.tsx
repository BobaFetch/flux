import React, { useState } from "react";

type Props = { title: string; count?: number };

export function Card({ title, count = 0 }: Props): JSX.Element {
  const [open, setOpen] = useState<boolean>(false);
  return (
    <section className="card" onClick={() => setOpen(!open)}>
      <h1>{title}</h1>
      {open && <p>Count: {count}</p>}
      <Footer />
    </section>
  );
}
