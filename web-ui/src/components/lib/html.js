import htm from '/vendor/htm.js';

class SafeHtml {
  constructor(value) {
    this.value = value;
  }
  toString() {
    return this.value;
  }
}

const VOID = new Set([
  'area',
  'base',
  'br',
  'col',
  'embed',
  'hr',
  'img',
  'input',
  'link',
  'meta',
  'param',
  'source',
  'track',
  'wbr',
]);

function esc(s) {
  return String(s)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;');
}

function escAttr(s) {
  return String(s).replace(/&/g, '&amp;').replace(/"/g, '&quot;');
}

function child(c) {
  if (c == null || c === false) return '';
  if (Array.isArray(c)) return c.map(child).join('');
  if (c instanceof SafeHtml) return c.value;
  if (typeof c === 'string') return esc(c);
  return esc(String(c));
}

function attr(k, v) {
  if (v == null || v === false) return '';
  if (k.startsWith('on')) return '';
  if (k === 'className') k = 'class';
  if (v === true) return ` ${k}`;
  return ` ${k}="${escAttr(v)}"`;
}

function serialize(tag, props, ...kids) {
  if (typeof tag === 'function') return tag(props || {}, ...kids);
  let s = `<${tag}`;
  if (props) for (const k in props) s += attr(k, props[k]);
  const joined = kids.map(child).join('');
  if (VOID.has(tag)) return new SafeHtml(`${s}>`);
  s += '>';
  s += joined;
  return new SafeHtml(`${s}</${tag}>`);
}

export const html = htm.bind(serialize);
export { SafeHtml };
