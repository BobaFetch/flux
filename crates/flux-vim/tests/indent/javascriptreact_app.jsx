import React from "react";

export function App({ items }) {
  const [open, setOpen] = React.useState(false);
  return (
    <div className="app">
      <h1>Title</h1>
      {items.map((item) => (
        <Item key={item.id} {...item} />
      ))}
      <button onClick={() => setOpen(!open)}>
        Toggle
      </button>
    </div>
  );
}
