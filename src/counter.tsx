"use client";

import React from "react";

export function Counter() {
  const [count, setCount] = React.useState(0);

  return (
    <button type="button" onClick={() => setCount((count) => count + 1)}>
      Client Counter: {count}
    </button>
  );
}
