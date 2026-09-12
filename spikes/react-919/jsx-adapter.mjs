import React from '@js/react';
export const Fragment = React.Fragment;
let vocabulary;
export function setVocabulary(value) { vocabulary = value; }
export function jsx(tag, props, ...children) {
  const type = typeof tag === 'string' ? vocabulary[tag] : tag;
  if(!type) throw Error(`Unknown spike vocabulary tag ${tag}`);
  return React.createElement(type, props, ...children);
}
export function jsxs(tag, props, children) { return jsx(tag, props, ...children); }

// Legacy live expression: React re-evaluates this during the component render.
export function live(read) { return read(); }
