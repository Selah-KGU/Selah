function sanitizeRichTextHtml(input) {
  var raw = String(input || '');
  if (!raw) return '';

  var hasTag = /<\/?[a-z][\s\S]*>/i.test(raw);
  if (!hasTag) return autoLinkEscaped(escapeHtml(raw)).replace(/\n/g, '<br>');

  var allowed = {
    a: true, b: true, strong: true, i: true, em: true, u: true,
    p: true, br: true, ul: true, ol: true, li: true,
    code: true, pre: true, blockquote: true,
    h1: true, h2: true, h3: true, h4: true, h5: true, h6: true,
    span: true, div: true, img: true
  };

  var tpl = document.createElement('template');
  tpl.innerHTML = raw;

  function safeHref(href) {
    var h = String(href || '').trim();
    if (!h) return '';
    var low = h.toLowerCase();
    if (low.startsWith('javascript:') || low.startsWith('data:') || low.startsWith('vbscript:')) return '';
    if (low.startsWith('http://') || low.startsWith('https://') || low.startsWith('mailto:') || low.startsWith('tel:') || h.startsWith('/') || h.startsWith('#')) return h;
    return '';
  }

  function sanitizeInlineStyle(styleText) {
    var rawStyle = String(styleText || '').trim();
    if (!rawStyle) return '';

    var allowedProps = {
      'font-weight': /^(normal|bold|[1-9]00)$/i,
      'font-style': /^(normal|italic|oblique)$/i,
      'text-decoration': /^(none|underline|line-through|overline)$/i,
      'text-align': /^(left|right|center|justify|start|end)$/i,
      'color': /^(#[0-9a-f]{3,8}|rgb\([^\)]{1,40}\)|rgba\([^\)]{1,50}\)|hsl\([^\)]{1,40}\)|hsla\([^\)]{1,50}\)|[a-z]{3,20})$/i,
      'background-color': /^(#[0-9a-f]{3,8}|rgb\([^\)]{1,40}\)|rgba\([^\)]{1,50}\)|hsl\([^\)]{1,40}\)|hsla\([^\)]{1,50}\)|[a-z]{3,20})$/i
    };

    var out = [];
    var parts = rawStyle.split(';');
    for (var pi = 0; pi < parts.length; pi++) {
      var seg = parts[pi];
      var idx = seg.indexOf(':');
      if (idx <= 0) continue;
      var key = seg.slice(0, idx).trim().toLowerCase();
      var val = seg.slice(idx + 1).trim();
      if (!key || !val) continue;
      if (val.indexOf('url(') >= 0 || val.indexOf('expression(') >= 0) continue;
      var rule = allowedProps[key];
      if (rule && rule.test(val)) out.push(key + ':' + val);
    }

    return out.join('; ');
  }

  function walk(node) {
    if (node.nodeType === Node.TEXT_NODE) {
      return autoLinkEscaped(escapeHtml(node.nodeValue || ''));
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return '';

    var tag = node.tagName.toLowerCase();
    if (tag === 'script' || tag === 'style' || tag === 'iframe' || tag === 'object' || tag === 'embed' || tag === 'svg') return '';

    var inner = '';
    for (var i = 0; i < node.childNodes.length; i++) inner += walk(node.childNodes[i]);

    if (!allowed[tag]) return inner;

    if (tag === 'a') {
      var href = safeHref(node.getAttribute('href'));
      if (!href) return inner;
      return '<a href="' + escapeHtml(href) + '" target="_blank" rel="noopener">' + inner + '</a>';
    }

    if (tag === 'img') {
      var rawSrc = String(node.getAttribute('src') || '').trim();
      var safeSrc = '';
      if (rawSrc.startsWith('https://') || /^data:image\/(png|jpe?g|gif|webp|svg\+xml|bmp|avif);base64,/i.test(rawSrc)) {
        safeSrc = rawSrc;
      }
      if (!safeSrc) return '';
      var altAttr = escapeHtml(String(node.getAttribute('alt') || ''));
      return '<img src="' + escapeHtml(safeSrc) + '" alt="' + altAttr + '" loading="lazy" data-lightbox="1">';
    }

    var styleAttr = '';
    if (tag === 'span' || tag === 'p' || tag === 'div'
      || tag === 'h1' || tag === 'h2' || tag === 'h3' || tag === 'h4' || tag === 'h5' || tag === 'h6') {
      var safeStyle = sanitizeInlineStyle(node.getAttribute('style'));
      if (safeStyle) styleAttr = ' style="' + escapeHtml(safeStyle) + '"';
    }

    return '<' + tag + styleAttr + '>' + inner + '</' + tag + '>';
  }

  var out = '';
  for (var i = 0; i < tpl.content.childNodes.length; i++) out += walk(tpl.content.childNodes[i]);
  return out;
}

function renderRichText(s, compact) {
  var html = sanitizeRichTextHtml(s);
  if (!compact) return html;
  return html;
}
