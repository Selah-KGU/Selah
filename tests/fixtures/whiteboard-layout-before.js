/* Whiteboard layout — shared between the in-app Live view and the standalone
 * markdown reader webview. Single source of truth; both consumers render the
 * returned shape but compute it through this module.
 *
 * Usage:
 *   var layout = window.WhiteboardLayout.compute(rawBoard, {
 *     fallbackBoardTitle: 'Knowledge Board',
 *     externalNodeLabel: '外部',    // used when external_source is empty
 *     topicIds: ['m1', 'm2']       // optional: render only these main topics
 *   });
 *   var topics = window.WhiteboardLayout.topics(rawBoard);  // [{id,label}]
 *
 *   layout === null when the board is unusable (less than 2 valid nodes).
 *   layout.nodes[i] = { id, label, detail, kind, role, parentId, sourceType,
 *                       sourceLabel, nodeType, x, y, chips }
 *   layout.edges[i] = { id, label, colorKind, colorSourceType,
 *                       x1, y1, x2, y2, cx, cy, lx, ly,
 *                       trunk, redundant }
 *   layout.stage   = { width, height } | null — pixel canvas the layout was
 *                    computed for; the renderer sizes the stage to match.
 *
 * Node x/y are 0..100 board coordinates. Edge x1/y1/x2/y2 and the cx/cy
 * quadratic-Bezier control point are in stage pixels (the SVG edge layer uses
 * a pixel viewBox), so renderers do `M x1 y1 Q cx cy x2 y2`. lx/ly are the
 * label centre in 0..100 board coordinates.
 * chips are term annotations folded into a structure node (drawn in-card).
 *
 * ── Normalization contract ────────────────────────────────────────────────
 * When board.normalized_by === 'backend' (schema_version >= 1), structural
 * fields (node_type, role, kind, parent_id, source_type) have already been
 * validated by parse_live_whiteboard and are passed through verbatim.
 * This module is then responsible ONLY for:
 *   - coordinate assignment (x/y via the Layered Topic Forest layout)
 *   - folding term nodes into their parent's chip list
 *   - edge geometry (Bézier control points, label placement)
 *
 * For legacy / demo / model-raw boards (normalized_by !== 'backend'), full
 * local normalization runs as before so heuristic defaults remain available.
 * ─────────────────────────────────────────────────────────────────────────
 */
(function (global) {
  'use strict';

  function clampBoardPoint(value, min, max) {
    if (min == null) min = 10;
    if (max == null) max = 90;
    return Math.round(Math.min(max, Math.max(min, value)) * 10) / 10;
  }

  function normalizeKind(kind) {
    var v = String(kind || 'support').toLowerCase();
    return (v === 'core' || v === 'result' || v === 'question') ? v : 'support';
  }

  function normalizeNodeType(nodeType) {
    var v = String(nodeType || '').toLowerCase();
    return (v === 'term' || v === 'terminology' || v === 'keyword' || v === 'small') ? 'term' : 'structure';
  }

  function normalizeRole(role, kind, parentId) {
    var v = String(role || '').toLowerCase();
    if (v === 'main' || v === 'primary' || v === 'trunk' || v === 'core') return 'main';
    if (v === 'branch' || v === 'detail' || v === 'leaf' || v === 'support') return 'branch';
    return normalizeKind(kind) === 'core' && !String(parentId || '').trim() ? 'main' : 'branch';
  }

  function legacyRole(kind) {
    var n = normalizeKind(kind);
    return (n === 'core' || n === 'result') ? 'main' : 'branch';
  }

  function normalizeSourceType(sourceType, externalSource) {
    var v = String(sourceType || '').toLowerCase();
    if (v === 'external' || v === 'outside' || v === 'reference') return 'external';
    return (externalSource && String(externalSource).trim()) ? 'external' : 'lecture';
  }

  function edgeColorSourceType(from, to) {
    return (from.sourceType === 'external' || to.sourceType === 'external') ? 'external' : 'lecture';
  }

  function edgeColorKind(from, to) {
    var a = normalizeKind(from.kind);
    var b = normalizeKind(to.kind);
    if (a === b) return 'support';
    if (a === 'question' || b === 'question') return 'question';
    if (a === 'result' || b === 'result') return 'result';
    return 'support';
  }

  function ellipsePoints(count, cx, cy, rx, ry) {
    var out = [];
    for (var i = 0; i < count; i++) {
      var a = -Math.PI / 2 + (i * Math.PI * 2) / count;
      out.push([
        Math.round((cx + Math.cos(a) * rx) * 10) / 10,
        Math.round((cy + Math.sin(a) * ry) * 10) / 10
      ]);
    }
    return out;
  }

  function comparePoints(count) {
    var left = Math.ceil(count / 2);
    var right = count - left;
    var side = function (items, x) {
      var out = [];
      for (var i = 0; i < items; i++) {
        var y = items === 1 ? 50 : 24 + (i * 52) / (items - 1);
        out.push([x, Math.round(y * 10) / 10]);
      }
      return out;
    };
    return side(left, 28).concat(side(right, 72));
  }

  function gridPoints(count, preferredCols, serpentine) {
    var cols = preferredCols || Math.min(4, Math.ceil(Math.sqrt(count)));
    var rows = Math.ceil(count / cols);
    var out = [];
    for (var i = 0; i < count; i++) {
      var row = Math.floor(i / cols);
      var colCount = Math.min(cols, count - row * cols);
      var base = i % cols;
      var col = serpentine && row % 2 === 1 ? colCount - 1 - base : base;
      var x = colCount === 1 ? 50 : 16 + (col * 68) / (colCount - 1);
      var y = rows === 1 ? 50 : 22 + (row * 56) / (rows - 1);
      out.push([Math.round(x * 10) / 10, Math.round(y * 10) / 10]);
    }
    return out;
  }

  function whiteboardPoints(count, layout) {
    var n = Math.max(2, count);
    if (layout === 'hub') {
      var arr = [[50, 50]].concat(ellipsePoints(n - 1, 50, 50, 34, 30));
      return arr.slice(0, count);
    }
    if (layout === 'compare') return comparePoints(n).slice(0, count);
    if (layout === 'cycle') return ellipsePoints(n, 50, 50, 34, 32).slice(0, count);
    if (layout === 'flow') return gridPoints(n, Math.min(4, n), true).slice(0, count);
    return gridPoints(n).slice(0, count);
  }

  function estimateLabelWidthEm(label) {
    // CJK / full-width ~1.0em, ASCII / half-width ~0.55em, plus 1.6em padding.
    var width = 1.6;
    for (var i = 0; i < label.length; i++) {
      var code = label.charCodeAt(i);
      width += code >= 0x3000 ? 1.0 : 0.55;
    }
    return width;
  }

  function rectOverlapArea(a, b) {
    var x = Math.max(0, Math.min(a.x2, b.x2) - Math.max(a.x1, b.x1));
    var y = Math.max(0, Math.min(a.y2, b.y2) - Math.max(a.y1, b.y1));
    return x * y;
  }

  function segmentsCross(ax, ay, bx, by, cx, cy, dx, dy) {
    function s(x) { return x > 0 ? 1 : x < 0 ? -1 : 0; }
    var d1 = s((bx - ax) * (cy - ay) - (by - ay) * (cx - ax));
    var d2 = s((bx - ax) * (dy - ay) - (by - ay) * (dx - ax));
    var d3 = s((dx - cx) * (ay - cy) - (dy - cy) * (ax - cx));
    var d4 = s((dx - cx) * (by - cy) - (dy - cy) * (bx - cx));
    return d1 !== d2 && d3 !== d4;
  }

  function rectSegmentIntersect(rect, seg) {
    if (seg.x1 >= rect.x1 && seg.x1 <= rect.x2 && seg.y1 >= rect.y1 && seg.y1 <= rect.y2) return true;
    if (seg.x2 >= rect.x1 && seg.x2 <= rect.x2 && seg.y2 >= rect.y1 && seg.y2 <= rect.y2) return true;
    return (
      segmentsCross(seg.x1, seg.y1, seg.x2, seg.y2, rect.x1, rect.y1, rect.x2, rect.y1) ||
      segmentsCross(seg.x1, seg.y1, seg.x2, seg.y2, rect.x2, rect.y1, rect.x2, rect.y2) ||
      segmentsCross(seg.x1, seg.y1, seg.x2, seg.y2, rect.x2, rect.y2, rect.x1, rect.y2) ||
      segmentsCross(seg.x1, seg.y1, seg.x2, seg.y2, rect.x1, rect.y2, rect.x1, rect.y1)
    );
  }

  function placeEdgeLabel(midX, midY, from, to, lw, lh, occupied, nodeRects, otherSegments, edgeIndex) {
    var dx = to.x - from.x;
    var dy = to.y - from.y;
    var length = Math.sqrt(dx * dx + dy * dy) || 1;
    var nx = -dy / length;
    var ny = dx / length;
    var tx = dx / length;
    var ty = dy / length;
    var normalOffsets = [0, 3.8, -3.8, 6.4, -6.4, 9, -9];
    var tangentOffsets = [0, 5.5, -5.5, 10, -10];
    var best = null;
    var bestScore = Infinity;
    for (var i = 0; i < normalOffsets.length; i++) {
      for (var j = 0; j < tangentOffsets.length; j++) {
        var normal = normalOffsets[i];
        var tangent = tangentOffsets[j];
        var x = clampBoardPoint(midX + nx * normal + tx * tangent, 6 + lw / 2, 94 - lw / 2);
        var y = clampBoardPoint(midY + ny * normal + ty * tangent, 7 + lh / 2, 93 - lh / 2);
        var cost = Math.abs(normal) * 1.6 + Math.abs(tangent) + (edgeIndex % 2 === 0 && normal < 0 ? 0.4 : 0);
        var rect = { x1: x - lw / 2, y1: y - lh / 2, x2: x + lw / 2, y2: y + lh / 2 };
        var score = cost;
        for (var k = 0; k < occupied.length; k++) score += rectOverlapArea(rect, occupied[k]) * 12;
        for (var m = 0; m < nodeRects.length; m++) score += rectOverlapArea(rect, nodeRects[m]) * 14;
        if (otherSegments) {
          for (var s = 0; s < otherSegments.length; s++) {
            if (rectSegmentIntersect(rect, otherSegments[s])) score += 3.2;
          }
        }
        if (score < bestScore) { bestScore = score; best = { x: x, y: y, rect: rect }; }
      }
    }
    if (!best) {
      var fx = clampBoardPoint(midX, 6 + lw / 2, 94 - lw / 2);
      var fy = clampBoardPoint(midY, 7 + lh / 2, 93 - lh / 2);
      best = { x: fx, y: fy, rect: { x1: fx - lw / 2, y1: fy - lh / 2, x2: fx + lw / 2, y2: fy + lh / 2 } };
    }
    occupied.push(best.rect);
    return { x: best.x, y: best.y };
  }

  // ── Layered Topic Forest layout ─────────────────────────────────────────
  // The board is a forest of topic trees (main → branches → …) plus a sparse
  // overlay of cross edges; term annotations are folded into parent chips
  // upstream. Each topic is laid out as a tidy tree: subtrees are packed by
  // contour (each sibling slid toward the block until its slots just touch —
  // van der Ploeg's non-layered result, computed by direct minimal-shift,
  // O(n²) which is irrelevant at board scale). Many-leaf fan-outs wrap into a
  // grid; orientation is per topic (shallow→top-down, deep→left-right). Topic
  // boxes are shelf-packed, then normalised to the 0..100 board space; the
  // pixel bounds become the stage-size hint.
  function computeForestLayout(nodes) {
    if (!nodes.length) return { points: {}, stage: null };

    // Slot = node box + surrounding gap, so touching slots leave a clean gap
    // between the cards. Height tracks the node's real content — detail-text
    // line estimate + chip rows — so a tall card never overlaps its neighbour.
    function slotOf(n) {
      var isMain = n.role === 'main';
      var chips = n.chips ? n.chips.length : 0;
      var chipH = chips ? chips * 20 + 8 : 0;
      var perLine = isMain ? 17 : 14;
      var detail = n.detail || '';
      var lines = detail ? Math.min(4, Math.ceil(detail.length / perLine)) : 0;
      var base = isMain ? 80 : 66;          // card height with the label only
      return { w: isMain ? 200 : 174, h: base + lines * 15 + chipH + 40 };
    }
    var GRID_MIN = 6;     // child count that triggers grid wrapping
    var TOPIC_GAP = 70;
    var MARGIN = 60;

    var byId = {};
    nodes.forEach(function (n) { byId[n.id] = n; });
    var mains = nodes.filter(function (n) { return n.role === 'main'; });
    var rootFallback = (mains[0] || nodes[0]).id;

    var childIds = {};
    nodes.forEach(function (n) { childIds[n.id] = []; });
    nodes.forEach(function (n) {
      if (n.role === 'main') return;
      var pid = (byId[n.parentId] && n.parentId !== n.id) ? n.parentId : rootFallback;
      if (childIds[pid]) childIds[pid].push(n.id);
    });

    // Structural depth — used only to pick orientation.
    function structuralDepth(id, seen) {
      seen = seen || {};
      if (seen[id]) return 1;
      seen[id] = true;
      var d = 1;
      childIds[id].forEach(function (cid) {
        if (byId[cid]) d = Math.max(d, 1 + structuralDepth(cid, seen));
      });
      return d;
    }

    // Combine child subtree-boxes into one block. `placed` entries carry x/y/w/h
    // (slot top-left + size) so contour packing can test real collisions.
    // Returns rootMin/rootMax: the child roots' centres along the main axis,
    // so the parent can be centred over them (tidy-tree centring).
    function arrangeBlock(childLayouts, orient) {
      var n = childLayouts.length;
      var placed = [];

      if (orient === 'TB' && n >= GRID_MIN) {
        var cellW = 0, cellH = 0;
        childLayouts.forEach(function (c) {
          cellW = Math.max(cellW, c.w); cellH = Math.max(cellH, c.h);
        });
        var cols = Math.max(1, Math.ceil(Math.sqrt(n * 1.5)));
        var rows = Math.ceil(n / cols);
        childLayouts.forEach(function (c, i) {
          var ox = (i % cols) * cellW + (cellW - c.w) / 2;
          var oy = Math.floor(i / cols) * cellH + (cellH - c.h) / 2;
          c.placed.forEach(function (p) {
            placed.push({ id: p.id, x: p.x + ox, y: p.y + oy, w: p.w, h: p.h });
          });
        });
        var gw = cols * cellW;
        return { w: gw, h: rows * cellH, placed: placed, rootMin: gw / 2, rootMax: gw / 2 };
      }

      // Contour packing: slide each child toward the accumulated block until
      // its slots just touch — sibling subtrees interleave into free space.
      var horizontal = orient !== 'LR';
      var block = [];
      var rootMin = 0, rootMax = 0;
      childLayouts.forEach(function (c, ci) {
        var shift = 0;
        for (var bi = 0; bi < block.length; bi++) {
          var a = block[bi];
          for (var pi = 0; pi < c.placed.length; pi++) {
            var b = c.placed[pi];
            if (horizontal) {
              if (b.y < a.y + a.h && a.y < b.y + b.h) {
                shift = Math.max(shift, a.x + a.w - b.x);
              }
            } else if (b.x < a.x + a.w && a.x < b.x + b.w) {
              shift = Math.max(shift, a.y + a.h - b.y);
            }
          }
        }
        c.placed.forEach(function (p) {
          var np = horizontal
            ? { id: p.id, x: p.x + shift, y: p.y, w: p.w, h: p.h }
            : { id: p.id, x: p.x, y: p.y + shift, w: p.w, h: p.h };
          placed.push(np);
          block.push(np);
        });
        var root = c.placed[0];
        var rc = horizontal ? root.x + shift + root.w / 2 : root.y + shift + root.h / 2;
        if (ci === 0) rootMin = rc;
        rootMax = rc;
      });
      var w = 0, h = 0;
      placed.forEach(function (p) { w = Math.max(w, p.x + p.w); h = Math.max(h, p.y + p.h); });
      return { w: w, h: h, placed: placed, rootMin: rootMin, rootMax: rootMax };
    }

    // Lay out the subtree rooted at `id`; placed x/y/w/h are slot rects,
    // relative to the subtree's own top-left corner. The parent is centred
    // over its children's roots.
    function layoutNode(id, orient, seen) {
      var slot = slotOf(byId[id]);
      var kids = (seen[id] ? [] : childIds[id]);
      seen[id] = true;
      if (!kids.length) {
        return { w: slot.w, h: slot.h, placed: [{ id: id, x: 0, y: 0, w: slot.w, h: slot.h }] };
      }
      var childLayouts = kids.map(function (cid) { return layoutNode(cid, orient, seen); });
      var block = arrangeBlock(childLayouts, orient);
      var mid = (block.rootMin + block.rootMax) / 2;
      var placed = [];
      var w, h;
      if (orient === 'LR') {
        var selfY0 = mid - slot.h / 2;
        var blockY = Math.max(0, -selfY0);
        var selfY = Math.max(0, selfY0);
        w = slot.w + block.w;
        h = Math.max(selfY + slot.h, blockY + block.h);
        placed.push({ id: id, x: 0, y: selfY, w: slot.w, h: slot.h });
        block.placed.forEach(function (p) {
          placed.push({ id: p.id, x: p.x + slot.w, y: p.y + blockY, w: p.w, h: p.h });
        });
      } else {
        var selfX0 = mid - slot.w / 2;
        var blockX = Math.max(0, -selfX0);
        var selfX = Math.max(0, selfX0);
        w = Math.max(selfX + slot.w, blockX + block.w);
        h = slot.h + block.h;
        placed.push({ id: id, x: selfX, y: 0, w: slot.w, h: slot.h });
        block.placed.forEach(function (p) {
          placed.push({ id: p.id, x: p.x + blockX, y: p.y + slot.h, w: p.w, h: p.h });
        });
      }
      return { w: w, h: h, placed: placed };
    }

    var topicRoots = mains.length ? mains : [nodes[0]];
    var topicBoxes = topicRoots.map(function (m) {
      var orient = structuralDepth(m.id) <= 2 ? 'TB' : 'LR';
      return layoutNode(m.id, orient, {});
    });

    // Shelf-pack the topic boxes into a roughly landscape area.
    var totalArea = 0;
    topicBoxes.forEach(function (b) { totalArea += b.w * b.h; });
    var rowLimit = Math.max(
      topicBoxes.reduce(function (mx, b) { return Math.max(mx, b.w); }, 0),
      Math.sqrt(totalArea) * 1.3
    );
    var abs = {};
    var cursorX = 0, cursorY = 0, rowH = 0, maxX = 0;
    topicBoxes.forEach(function (b) {
      if (cursorX > 0 && cursorX + b.w > rowLimit) {
        cursorX = 0; cursorY += rowH + TOPIC_GAP; rowH = 0;
      }
      b.placed.forEach(function (p) {
        abs[p.id] = { x: p.x + cursorX, y: p.y + cursorY };
      });
      cursorX += b.w + TOPIC_GAP;
      rowH = Math.max(rowH, b.h);
      maxX = Math.max(maxX, cursorX - TOPIC_GAP);
    });
    var stageW = maxX + MARGIN * 2;
    var stageH = cursorY + rowH + MARGIN * 2;

    // Normalise slot centres to 0..100; the renderer scales them to the stage.
    var points = {};
    nodes.forEach(function (n) {
      var a = abs[n.id];
      if (!a) { points[n.id] = [50, 50]; return; }
      var slot = slotOf(n);
      var cx = a.x + slot.w / 2 + MARGIN;
      var cy = a.y + slot.h / 2 + MARGIN;
      points[n.id] = [
        Math.round((cx / stageW) * 1000) / 10,
        Math.round((cy / stageH) * 1000) / 10
      ];
    });
    return { points: points, stage: { width: Math.round(stageW), height: Math.round(stageH) } };
  }

  // Enumerate the main ("topic") nodes of a board so a renderer can offer a
  // topic switcher. Returns [{ id, label }] in board order; empty when the
  // board has no explicit hierarchy (nothing to switch between).
  function topics(board) {
    if (!board || typeof board !== 'object') return [];
    var rawNodes = (Array.isArray(board.nodes) ? board.nodes : [])
      .filter(function (n) { return n && typeof n.label === 'string' && n.label.trim(); });
    if (rawNodes.length < 2) return [];
    var backendNormalized = !!(board.normalized_by === 'backend');
    var hasExplicitHierarchy = backendNormalized || rawNodes.some(function (n) {
      return (n.role && String(n.role).trim()) || (n.parent_id && String(n.parent_id).trim());
    });
    if (!hasExplicitHierarchy) return [];
    var out = [];
    rawNodes.forEach(function (n, i) {
      var role = backendNormalized
        ? String(n.role || 'branch')
        : normalizeRole(n.role, n.kind, n.parent_id);
      if (role === 'main') {
        out.push({ id: (n.id && String(n.id)) || ('n' + (i + 1)), label: String(n.label).trim() });
      }
    });
    return out;
  }

  function compute(board, opts) {
    if (!board || typeof board !== 'object') return null;
    opts = opts || {};
    var fallbackTitle = opts.fallbackBoardTitle || 'Knowledge Board';
    var externalLabel = opts.externalNodeLabel || '外部';

    var rawNodes = (Array.isArray(board.nodes) ? board.nodes : [])
      .filter(function (n) { return n && typeof n.label === 'string' && n.label.trim(); });
    if (rawNodes.length < 2) return null;

    // Fast-path flag: backend-normalised boards (schema_version ≥ 1) have
    // already had structural fields validated. Skip re-normalisation and trust
    // the values directly; only layout / geometry passes run below.
    var backendNormalized = !!(board.normalized_by === 'backend');

    var hasExplicitHierarchy = backendNormalized || rawNodes.some(function (n) {
      return (n.role && String(n.role).trim()) || (n.parent_id && String(n.parent_id).trim());
    });

    var drafts = rawNodes.map(function (n, i) {
      var externalSource = n.external_source ? String(n.external_source).trim() : '';
      return {
        id: (n.id && String(n.id)) || ('n' + (i + 1)),
        label: String(n.label).trim(),
        detail: (n.detail ? String(n.detail).trim() : ''),
        nodeType: backendNormalized ? String(n.node_type || 'structure') : normalizeNodeType(n.node_type),
        kind: backendNormalized ? String(n.kind || 'support') : normalizeKind(n.kind),
        role: backendNormalized
          ? String(n.role || 'branch')
          : (hasExplicitHierarchy ? normalizeRole(n.role, n.kind, n.parent_id) : legacyRole(n.kind)),
        parentId: (n.parent_id ? String(n.parent_id).trim() : ''),
        sourceType: backendNormalized ? String(n.source_type || 'lecture') : normalizeSourceType(n.source_type, n.external_source),
        sourceLabel: externalSource || externalLabel
      };
    });
    // Backend-normalised boards already have correct node_type/role/kind.
    // For legacy / raw boards, force term nodes to the canonical sub-type values.
    if (!backendNormalized) {
      drafts.forEach(function (n) {
        if (n.nodeType !== 'term') return;
        n.kind = 'support';
        n.role = 'branch';
      });
    }

    var points;
    var stageHint = null;
    if (hasExplicitHierarchy) {
      // For backend-normalised boards, structural constraints (main existence,
      // parent-ID validity) are already guaranteed — skip redundant fixup passes
      // and go straight to layout computation.
      if (!backendNormalized) {
        if (!drafts.some(function (n) { return n.role === 'main'; })) {
          drafts[0].role = 'main';
        }
        var mainIds = {};
        drafts.forEach(function (n) { if (n.role === 'main') mainIds[n.id] = true; });
        var structureIds = {};
        drafts.forEach(function (n) { if (n.nodeType !== 'term') structureIds[n.id] = true; });
        var fallbackMain = null;
        for (var fi = 0; fi < drafts.length; fi++) {
          if (drafts[fi].role === 'main') { fallbackMain = drafts[fi]; break; }
        }
        var fallbackMainId = fallbackMain ? fallbackMain.id : drafts[0].id;
        drafts.forEach(function (n) {
          if (n.role === 'main') {
            n.parentId = '';
          } else if (!mainIds[n.parentId] || n.parentId === n.id) {
            if (n.nodeType === 'term') {
              n.parentId = structureIds[n.parentId] && n.parentId !== n.id ? n.parentId : '';
            } else {
              n.parentId = fallbackMainId;
            }
          }
        });
      }
      // Topic filter: when the caller requests a subset of main topics, keep
      // only nodes that belong to one of them — a node belongs to the nearest
      // main ancestor on its parent chain. Edges referencing dropped nodes are
      // skipped automatically downstream (byId lookups fail for missing ids).
      var topicIds = opts && Array.isArray(opts.topicIds) ? opts.topicIds : null;
      if (topicIds && topicIds.length) {
        var keepTopic = {};
        topicIds.forEach(function (id) { keepTopic[String(id)] = true; });
        var draftById = {};
        drafts.forEach(function (d) { draftById[d.id] = d; });
        drafts = drafts.filter(function (d) {
          var seen = {};
          var cur = d;
          while (cur && !seen[cur.id]) {
            if (cur.role === 'main') return !!keepTopic[cur.id];
            seen[cur.id] = true;
            cur = draftById[cur.parentId];
          }
          return false;
        });
      }
      // Fold term nodes into their parent's chip list. Terms are annotations,
      // not first-class graph nodes — folding them keeps the layout to the
      // structural skeleton (≈40% fewer nodes) and renders them as compact
      // chips inside the parent card instead of scattered pills.
      var foldById = {};
      drafts.forEach(function (d) { foldById[d.id] = d; });
      var structureDrafts = [];
      var orphanTerms = [];
      drafts.forEach(function (d) {
        if (d.nodeType !== 'term') { structureDrafts.push(d); return; }
        var parent = foldById[d.parentId];
        var chip = { label: d.label, detail: d.detail, sourceType: d.sourceType, sourceLabel: d.sourceLabel };
        if (parent && parent.nodeType !== 'term') {
          (parent.chips = parent.chips || []).push(chip);
        } else {
          orphanTerms.push(chip);
        }
      });
      if (orphanTerms.length && structureDrafts.length) {
        var chipHost = null;
        for (var ci = 0; ci < structureDrafts.length; ci++) {
          if (structureDrafts[ci].role === 'main') { chipHost = structureDrafts[ci]; break; }
        }
        chipHost = chipHost || structureDrafts[0];
        chipHost.chips = (chipHost.chips || []).concat(orphanTerms);
      }
      drafts = structureDrafts;
      var ltf = computeForestLayout(drafts);
      points = ltf.points;
      stageHint = ltf.stage;
    } else {
      var fallbackPoints = whiteboardPoints(drafts.length, String(board.layout || 'grid').toLowerCase());
      points = {};
      drafts.forEach(function (n, i) { points[n.id] = fallbackPoints[i] || [50, 50]; });
      // Legacy non-hierarchical boards still get a pixel stage so edges are
      // drawn in pixel space (the renderer's SVG viewBox / stroke width assume
      // pixels — a 0..100 viewBox would render edges far too thick).
      stageHint = { width: 1100, height: 740 };
    }

    var nodes = drafts.map(function (n) {
      var pt = points[n.id] || [50, 50];
      n.x = pt[0];
      n.y = pt[1];
      return n;
    });

    var byId = {};
    nodes.forEach(function (n) { byId[n.id] = n; });

    var occupiedLabelRects = [];
    var nodeLabelRects = nodes.map(function (n) {
      var halfW = n.role === 'main' ? 7.2 : 6.0;
      var halfH = n.role === 'main' ? 5.7 : 5.0;
      return { x1: n.x - halfW, y1: n.y - halfH, x2: n.x + halfW, y2: n.y + halfH };
    });

    // Term nodes were folded into chips upstream, so every layout node is a
    // structure node — edges are simply the board's edges between them.
    var rawEdges = Array.isArray(board.edges) ? board.edges : [];
    var validEdges = [];
    rawEdges.forEach(function (e, i) {
      if (!e) return;
      var from = byId[e.from];
      var to = byId[e.to];
      if (!from || !to || from.id === to.id) return;
      validEdges.push({ raw: e, index: i, from: from, to: to });
    });
    var edgeAdj = {};
    validEdges.forEach(function (ve) {
      (edgeAdj[ve.from.id] = edgeAdj[ve.from.id] || {})[ve.to.id] = true;
      (edgeAdj[ve.to.id] = edgeAdj[ve.to.id] || {})[ve.from.id] = true;
    });
    function isRedundant(aId, bId) {
      var an = edgeAdj[aId];
      var bn = edgeAdj[bId];
      if (!an || !bn) return false;
      for (var mid in an) if (mid !== bId && bn[mid]) return true;
      return false;
    }

    // 0..100 inset endpoints — used for label placement (board-space scoring)
    // and for the label-collision segment list.
    var edgeGeoms = validEdges.map(function (ve) {
      var from = ve.from, to = ve.to;
      var trunk = (from.role === 'main' && to.role === 'main') || from.parentId === to.id || to.parentId === from.id;
      var redundant = !trunk && isRedundant(from.id, to.id);
      var dx = to.x - from.x, dy = to.y - from.y;
      var len = Math.sqrt(dx * dx + dy * dy) || 1;
      var insetDist = Math.min(5.5, len * 0.18);
      var ux = dx / len, uy = dy / len;
      return {
        raw: ve.raw, index: ve.index, from: from, to: to, trunk: trunk, redundant: redundant,
        ix1: from.x + ux * insetDist, iy1: from.y + uy * insetDist,
        ix2: to.x - ux * insetDist, iy2: to.y - uy * insetDist
      };
    });
    var allSegments = edgeGeoms.map(function (g) {
      return { x1: g.ix1, y1: g.iy1, x2: g.ix2, y2: g.iy2 };
    });
    var edges = edgeGeoms.map(function (geom, gi) {
      var label = geom.raw.label ? String(geom.raw.label).trim() : '';
      var labelWidth = label ? clampBoardPoint(estimateLabelWidthEm(label), 5, 13.2) : 0;
      var labelHeight = 3.3;

      // Path geometry in stage pixels: the SVG layer uses a pixel viewBox so
      // the curve is drawn undistorted (the 0..100 space is non-square once
      // mapped to the stage).
      var sw = stageHint ? stageHint.width : 100;
      var sh = stageHint ? stageHint.height : 100;
      var fpx = geom.from.x / 100 * sw, fpy = geom.from.y / 100 * sh;
      var tpx = geom.to.x / 100 * sw, tpy = geom.to.y / 100 * sh;
      var pdx = tpx - fpx, pdy = tpy - fpy;
      var plen = Math.sqrt(pdx * pdx + pdy * pdy) || 1;
      var pinset = Math.min(46, Math.max(8, plen * 0.16));
      var pux = pdx / plen, puy = pdy / plen;
      var px1 = fpx + pux * pinset, py1 = fpy + puy * pinset;
      var px2 = tpx - pux * pinset, py2 = tpy - puy * pinset;
      var pnx = -pdy / plen, pny = pdx / plen;
      var pcurveSign = geom.index % 2 === 0 ? 1 : -1;
      // Gentle bow, scaled by length but capped so long cross-edges don't swoop.
      var pcurveMag = Math.min(
        geom.trunk ? 32 : 58,
        plen * (geom.trunk ? 0.05 : (geom.redundant ? 0.08 : 0.11))
      );
      var pcx = (px1 + px2) / 2 + pnx * pcurveSign * pcurveMag;
      var pcy = (py1 + py2) / 2 + pny * pcurveSign * pcurveMag;

      // Label anchor = the actual (pixel) curve's quarter-point, mapped back
      // to 0..100 so placeEdgeLabel scores it against the curve really drawn.
      // The label is positioned by percentage, so a point maps with no
      // aspect distortion (only SVG shapes stretch, not point positions).
      var anchorX = ((px1 + 2 * pcx + px2) / 4) / sw * 100;
      var anchorY = ((py1 + 2 * pcy + py2) / 4) / sh * 100;
      var otherSegs = [];
      for (var oi = 0; oi < allSegments.length; oi++) if (oi !== gi) otherSegs.push(allSegments[oi]);
      var lp = label ? placeEdgeLabel(
        anchorX, anchorY,
        { x: geom.ix1, y: geom.iy1 }, { x: geom.ix2, y: geom.iy2 },
        labelWidth, labelHeight,
        occupiedLabelRects, nodeLabelRects,
        otherSegs,
        geom.index
      ) : { x: anchorX, y: anchorY };
      return {
        id: geom.from.id + '-' + geom.to.id + '-' + geom.index,
        from: geom.from.id,
        to: geom.to.id,
        label: label,
        colorKind: edgeColorKind(geom.from, geom.to),
        colorSourceType: edgeColorSourceType(geom.from, geom.to),
        x1: px1, y1: py1,
        x2: px2, y2: py2,
        cx: pcx, cy: pcy,
        lx: lp.x, ly: lp.y,
        trunk: geom.trunk,
        redundant: geom.redundant
      };
    });

    return {
      title: (board.title ? String(board.title).trim() : '') || fallbackTitle,
      nodes: nodes,
      edges: edges,
      stage: stageHint
    };
  }

  global.WhiteboardLayout = { compute: compute, topics: topics };
})(typeof window !== 'undefined' ? window : globalThis);
