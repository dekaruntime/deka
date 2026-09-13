import {
  createContext,
  createElement as h,
  useContext,
  useEffect,
  useState,
} from "@js/react";
import { jsx, jsxs } from "@js/react/jsx-runtime";
import { renderToString } from "@js/react-dom/server";

const Theme = createContext("light");

function Counter() {
  const [n] = useState(7);
  return jsx("button", {
    id: "counter",
    "data-island": "counter",
    children: String(n),
  });
}

function EffectProbe() {
  const [label] = useState("ssr");
  useEffect(() => {
    void label;
  }, [label]);
  return jsx("span", { id: "effect", children: label });
}

function Themed() {
  const theme = useContext(Theme);
  return jsx("p", { id: "theme", "data-theme": theme, children: theme });
}

function App() {
  return jsx(Theme.Provider, {
    value: "dark",
    children: jsxs("main", {
      children: [h(Counter), h(EffectProbe), h(Themed)],
    }),
  });
}

globalThis.app = {
  async fetch() {
    const html = renderToString(jsx(App, {}));
    return new Response(html, {
      headers: { "content-type": "text/html; charset=utf-8" },
    });
  },
};
