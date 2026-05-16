/* global importScripts, marked */
importScripts("marked.umd.js");

self.onmessage = function (event) {
  var id = event.data && event.data.id;
  var markdown = String((event.data && event.data.markdown) || "");
  try {
    if (typeof marked !== "undefined") {
      marked.setOptions({ gfm: true, breaks: false });
      self.postMessage({ id: id, html: marked.parse(markdown) });
    } else {
      self.postMessage({ id: id, html: "<pre>" + escapeHtml(markdown) + "</pre>" });
    }
  } catch (error) {
    self.postMessage({ id: id, error: error && error.message ? error.message : String(error) });
  }
};

function escapeHtml(value) {
  return String(value).replace(/[&<>"']/g, function (c) {
    return ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", "\"": "&quot;", "'": "&#39;" })[c];
  });
}
