var UNIVERSITY_BASES = {
  luna: 'https://luna.kwansei.ac.jp',
  kwic: 'https://kwic.kwansei.ac.jp',
  kgc: 'https://kg-course.kwansei.ac.jp'
};
var AUTO_LINK_RE = /(https?:\/\/[^\s<)\]]+|\/(?:lms|portal|uniasv2)\/[^\s<)\]]+)/g;
var LINK_MATCH_IGNORED_PARAMS = {
  _cid: true,
  _csrf: true,
  screen: true,
  directLink: true,
  pageViewListNum: true,
  selectCategoryCd: true
};

function guessBaseOriginForHref(href, currentMode) {
  var raw = decodeHtmlEntities(href).trim();
  if (raw.indexOf('/portal/') === 0) return UNIVERSITY_BASES.kwic;
  if (raw.indexOf('/uniasv2/') === 0) return UNIVERSITY_BASES.kgc;
  if (raw.indexOf('/lms/') === 0) return UNIVERSITY_BASES.luna;
  if (currentMode === 'kwic') return UNIVERSITY_BASES.kwic;
  if (currentMode === 'kgc' || currentMode === 'syllabus') return UNIVERSITY_BASES.kgc;
  return UNIVERSITY_BASES.luna;
}

function linkifyEscapedText(escaped) {
  return String(escaped || '').replace(AUTO_LINK_RE, function(raw) {
    var href = escapeHtml(raw);
    return '<a href="' + href + '" target="_blank" rel="noopener" style="color:var(--accent);text-decoration:underline;word-break:break-all;">' + raw + '</a>';
  });
}

function linkifyText(s) {
  return linkifyEscapedText(escapeHtml(s));
}

function autoLinkEscaped(escaped) {
  return linkifyEscapedText(escaped);
}

function resolveUniversityUrl(href) {
  var raw = decodeHtmlEntities(href).trim();
  if (!raw || raw === '#') return null;
  var mode = new URLSearchParams(window.location.search).get('mode') || '';
  try {
    if (/^https?:\/\//i.test(raw)) return new URL(raw);
    return new URL(raw, guessBaseOriginForHref(raw, mode));
  } catch (e) {
    return null;
  }
}

function detectUniversityService(url) {
  if (!url) return '';
  var host = (url.hostname || '').toLowerCase();
  var path = url.pathname || '';
  if (host.indexOf('luna.kwansei.ac.jp') >= 0 || path.indexOf('/lms/') === 0) return 'luna';
  if (host.indexOf('kwic.kwansei.ac.jp') >= 0 || path.indexOf('/portal/') === 0) return 'kwic';
  if (host.indexOf('kg-course.kwansei.ac.jp') >= 0 || path.indexOf('/uniasv2/') === 0) return 'kgc';
  return '';
}

function normalizeMatchText(s) {
  return String(s || '')
    .toLowerCase()
    .replace(/[\s\u3000\u00a0]+/g, '')
    .replace(/[|｜:：()（）【】「」『』\[\]<>＜＞・,，.．]/g, '');
}

function titlesLooselyMatch(a, b) {
  var na = normalizeMatchText(a);
  var nb = normalizeMatchText(b);
  if (!na || !nb) return false;
  return na === nb || na.indexOf(nb) >= 0 || nb.indexOf(na) >= 0;
}

function exactParam(url, key) {
  return (url && url.searchParams && url.searchParams.get(key)) || '';
}

function buildUrlMatchKey(input) {
  var url = input instanceof URL ? input : resolveUniversityUrl(input);
  if (!url) return '';
  var params = Array.from(url.searchParams.entries())
    .filter(function(pair) { return !LINK_MATCH_IGNORED_PARAMS[pair[0]]; })
    .sort(function(a, b) {
      return a[0] === b[0]
        ? String(a[1]).localeCompare(String(b[1]))
        : String(a[0]).localeCompare(String(b[0]));
    });
  var search = params.map(function(pair) {
    return encodeURIComponent(pair[0]) + '=' + encodeURIComponent(pair[1]);
  }).join('&');
  return url.pathname + (search ? '?' + search : '');
}

function sameCoreUrl(a, b) {
  var ka = buildUrlMatchKey(a);
  var kb = buildUrlMatchKey(b);
  return !!ka && ka === kb;
}
