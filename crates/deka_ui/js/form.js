// ui/form — progressive enhancement. Renders a plain HTML <form>. Zero JS.

import { jsx } from "./jsx.js";

export function Form(props = {}) {
  const { children, ...attributes } = props;
  return jsx("form", attributes, children);
}
