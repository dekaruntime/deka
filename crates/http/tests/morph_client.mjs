// Executes the shipped morph client's island-signature guard against a
// minimal fake DOM (same pattern as refresh_client.mjs): the html-update
// morph must full-reload when a server-component edit changes an island's
// props/directive/set/order, and must keep morphing when only volatile
// marker cache tokens differ.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('../src/hmr_client/morph.js', import.meta.url), 'utf8');
const moduleUrl = code => `data:text/javascript;base64,${Buffer.from(code).toString('base64')}#`;
const morph = await import(moduleUrl(
  `${source}\nexport { collectIslandSignatures, islandSignaturesEqual };`,
));

// --- minimal fake DOM: only what the signature collector touches -----------
function link(parent, kids) {
  for (let i = 0; i < kids.length; i++) {
    kids[i].nextSibling = kids[i + 1] || null;
  }
  parent.firstChild = kids[0] || null;
}

function fragment(...kids) {
  const node = { nodeType: 11, firstChild: null };
  link(node, kids.flat());
  return node;
}

function element(tag, attrs = {}, ...kids) {
  const node = {
    nodeType: 1,
    tagName: tag.toUpperCase(),
    attrs,
    getAttribute(name) {
      return Object.prototype.hasOwnProperty.call(attrs, name) ? attrs[name] : null;
    },
    hasAttribute(name) {
      return Object.prototype.hasOwnProperty.call(attrs, name);
    },
    firstChild: null,
    nextSibling: null,
  };
  link(node, kids.flat());
  return node;
}

function comment(data) {
  return { nodeType: 8, data, firstChild: null, nextSibling: null };
}

function text(data) {
  return { nodeType: 3, data, firstChild: null, nextSibling: null };
}

// --- fixtures ----------------------------------------------------------------
const counter = (directive, props) =>
  element('deka-island', {
    'data-deka-island': 'Counter',
    'data-deka-directive': directive,
    'data-deka-props': JSON.stringify(props),
  });

const islandRange = (name, directive, props, cache, body) => [
  comment(`deka-island start:${name} directive:${directive} props:${props}${cache ? ` cache:${cache}` : ''}`),
  body,
  comment(`deka-island end:${name}`),
];

const sigs = root => morph.collectIslandSignatures(root);
const equal = (a, b) => morph.islandSignaturesEqual(sigs(a), sigs(b));

// Element-form islands: identical sequences compare equal.
assert.ok(
  equal(
    fragment(element('div', {}, counter('load', {}), text('a'))),
    fragment(element('div', {}, counter('load', {}), text('b'))),
  ),
  'a server-text edit with untouched islands must keep the morph path',
);

// Props change -> unequal (must reload).
assert.ok(
  !equal(
    fragment(counter('load', {})),
    fragment(counter('load', { greeting: 'hi' })),
  ),
  'an island props change must leave the morph path',
);

// Directive change -> unequal.
assert.ok(
  !equal(
    fragment(counter('load', {})),
    fragment(counter('idle', {})),
  ),
  'an island directive change must leave the morph path',
);

// Island added / removed / reordered -> unequal.
assert.ok(
  !equal(
    fragment(counter('load', {})),
    fragment(counter('load', {}), counter('load', {})),
  ),
  'an added island must leave the morph path',
);
assert.ok(
  !equal(
    fragment(counter('load', {}), counter('load', {})),
    fragment(counter('load', {})),
  ),
  'a removed island must leave the morph path',
);
assert.ok(
  !equal(
    fragment(counter('load', {}), counter('idle', {})),
    fragment(counter('idle', {}), counter('load', {})),
  ),
  'a reordered island sequence must leave the morph path',
);

// Comment-marker islands: same contract.
assert.ok(
  equal(
    fragment(...islandRange('Q291bnRlcg==', 'bG9hZA==', 'e30=', 'D:1', element('button'))),
    fragment(...islandRange('Q291bnRlcg==', 'bG9hZA==', 'e30=', 'D:9', element('button'))),
  ),
  'a volatile cache-token change must keep the morph path',
);
assert.ok(
  !equal(
    fragment(...islandRange('Q291bnRlcg==', 'bG9hZA==', 'e30=', 'D:1', element('button'))),
    fragment(...islandRange('Q291bnRlcg==', 'aWRsZQ==', 'e30=', 'D:1', element('button'))),
  ),
  'a comment-marker directive change must leave the morph path',
);
assert.ok(
  !equal(
    fragment(...islandRange('Q291bnRlcg==', 'bG9hZA==', 'e30=', 'D:1', element('button'))),
    fragment(...islandRange('Q291bnRlcg==', 'bG9hZA==', 'eyJhIjoxfQ==', 'D:1', element('button'))),
  ),
  'a comment-marker props change must leave the morph path',
);

console.log('morph client island-signature contract passed');
