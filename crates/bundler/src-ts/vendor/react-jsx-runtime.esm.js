/**
 * Bundled by jsDelivr using Rollup v2.79.2 and Terser v5.39.0.
 * Original file: /npm/react@18.3.1/jsx-runtime.js
 *
 * Modified to use local react import instead of CDN
 */
import r from"react";var e={exports:{}},o={},t=r,s=Symbol.for("react.element"),p=Symbol.for("react.fragment"),n=Object.prototype.hasOwnProperty,a=t.__SECRET_INTERNALS_DO_NOT_USE_OR_YOU_WILL_BE_FIRED.ReactCurrentOwner,f={key:!0,ref:!0,__self:!0,__source:!0};function _(r,e,o){var t,p={},_=null,l=null;for(t in void 0!==o&&(_=""+o),void 0!==e.key&&(_=""+e.key),void 0!==e.ref&&(l=e.ref),e)n.call(e,t)&&!f.hasOwnProperty(t)&&(p[t]=e[t]);if(r&&r.defaultProps)for(t in e=r.defaultProps)void 0===p[t]&&(p[t]=e[t]);return{$$typeof:s,type:r,key:_,ref:l,props:p,_owner:a.current}}o.Fragment=p,o.jsx=_,o.jsxs=_,e.exports=o;var l=e.exports,x=e.exports.Fragment,y=e.exports.jsx,m=e.exports.jsxs;export{x as Fragment,l as default,y as jsx,m as jsxs};