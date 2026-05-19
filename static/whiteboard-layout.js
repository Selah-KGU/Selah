/* Whiteboard layout — shared between the in-app Live view and the standalone
 * markdown reader webview. Single source of truth; both consumers render the
 * returned shape but compute it through this module.
 *
 * Usage:
 *   var layout = window.WhiteboardLayout.compute(rawBoard, {
 *     maxNodes: 18,                 // safety cap
 *     maxEdges: 24,                 // safety cap
 *     fallbackBoardTitle: 'Knowledge Board',
 *     externalNodeLabel: '外部'     // used when external_source is empty
 *   });
 *
 *   layout === null when the board is unusable (less than 2 valid nodes).
 *   layout.nodes[i] = { id, label, detail, kind, role, parentId, sourceType,
 *                       sourceLabel, x, y }
 *   layout.edges[i] = { id, label, colorKind, colorSourceType,
 *                       x1, y1, x2, y2, cx, cy, lx, ly,
 *                       labelWidth, trunk, redundant }
 *
 * x/y are 0..100 board coordinates. cx/cy is the quadratic Bezier control
 * point so renderers do `M x1 y1 Q cx cy x2 y2`. lx/ly are the label centre.
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

  function radialMainAnchors(count, layout) {
    if (count <= 0) return [];
    if (layout === 'compare' && count === 2) {
      return [
        { angle: Math.PI, point: [35, 48] },
        { angle: 0, point: [65, 48] }
      ];
    }
    if (count === 1) return [{ angle: -Math.PI / 2, point: [50, 48] }];
    var out = [];
    for (var i = 0; i < count; i++) {
      var angle = -Math.PI / 2 + (i * Math.PI * 2) / count;
      out.push({
        angle: angle,
        point: [
          Math.round((50 + Math.cos(angle) * 19) * 10) / 10,
          Math.round((50 + Math.sin(angle) * 15) * 10) / 10
        ]
      });
    }
    return out;
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

  function rebalanceBranchesByBarycenter(branchesByParent, edges, points) {
    Object.keys(branchesByParent).forEach(function (parentId) {
      var branches = branchesByParent[parentId];
      if (!branches || branches.length < 2) return;
      var parentPos = points[parentId];
      if (!parentPos) return;
      var siblingIds = {};
      branches.forEach(function (b) { siblingIds[b.id] = true; });

      var slots = [];
      branches.forEach(function (b) {
        var p = points[b.id];
        if (!p) return;
        var dx = p[0] - parentPos[0];
        var dy = p[1] - parentPos[1];
        slots.push({ angle: Math.atan2(dy, dx), radius: Math.sqrt(dx * dx + dy * dy) || 40 });
      });
      if (slots.length < 2) return;
      slots.sort(function (a, b) { return a.angle - b.angle; });

      var preferred = branches.map(function (b) {
        var sx = 0, sy = 0, count = 0;
        for (var i = 0; i < edges.length; i++) {
          var e = edges[i];
          var otherId = null;
          if (e.from === b.id) otherId = e.to;
          else if (e.to === b.id) otherId = e.from;
          if (!otherId) continue;
          if (otherId === parentId || siblingIds[otherId]) continue;
          var op = points[otherId];
          if (!op) continue;
          sx += op[0] - parentPos[0];
          sy += op[1] - parentPos[1];
          count++;
        }
        var p = points[b.id];
        var fallback = p ? Math.atan2(p[1] - parentPos[1], p[0] - parentPos[0]) : 0;
        return { id: b.id, angle: count > 0 ? Math.atan2(sy, sx) : fallback };
      });
      preferred.sort(function (a, b) { return a.angle - b.angle; });

      preferred.forEach(function (entry, i) {
        var slot = slots[i];
        points[entry.id] = [
          clampBoardPoint(parentPos[0] + Math.cos(slot.angle) * slot.radius, 6, 94),
          clampBoardPoint(parentPos[1] + Math.sin(slot.angle) * slot.radius, 7, 93)
        ];
      });
    });
  }

  function relaxPoints(nodes, edges, initialPoints, mainAnchors, mains, iterations) {
    if (iterations == null) iterations = 130;
    var nodeIds = {};
    var nodeById = {};
    var points = {};
    nodes.forEach(function (n) {
      nodeIds[n.id] = true;
      nodeById[n.id] = n;
      points[n.id] = initialPoints[n.id] || [50, 50];
    });
    var anchorById = {};
    mains.forEach(function (n, i) {
      anchorById[n.id] = (mainAnchors[i] && mainAnchors[i].point) || [50, 46];
    });
    var springs = [];
    edges.forEach(function (e) {
      if (!e || !nodeIds[e.from] || !nodeIds[e.to] || e.from === e.to) return;
      var from = nodeById[e.from];
      var to = nodeById[e.to];
      var parentLink = (from && from.parentId === e.to) || (to && to.parentId === e.from);
      springs.push({ from: e.from, to: e.to, ideal: parentLink ? 23 : 22, strength: parentLink ? 0.09 : 0.085 });
    });
    nodes.forEach(function (n) {
      if (n.role === 'main' || !nodeById[n.parentId]) return;
      var exists = springs.some(function (s) {
        return (s.from === n.id && s.to === n.parentId) || (s.to === n.id && s.from === n.parentId);
      });
      if (!exists) springs.push({ from: n.id, to: n.parentId, ideal: 24, strength: 0.08 });
    });

    var adjacency = {};
    springs.forEach(function (s) {
      (adjacency[s.from] = adjacency[s.from] || []).push(s.to);
      (adjacency[s.to] = adjacency[s.to] || []).push(s.from);
    });

    for (var step = 0; step < iterations; step++) {
      var delta = {};
      nodes.forEach(function (n) { delta[n.id] = [0, 0]; });
      springs.forEach(function (s) {
        var a = points[s.from];
        var b = points[s.to];
        if (!a || !b) return;
        var dx = b[0] - a[0];
        var dy = b[1] - a[1];
        var distance = Math.sqrt(dx * dx + dy * dy) || 0.001;
        var force = (distance - s.ideal) * s.strength;
        var fx = (dx / distance) * force;
        var fy = (dy / distance) * force;
        var fromMain = nodeById[s.from] && nodeById[s.from].role === 'main';
        var toMain = nodeById[s.to] && nodeById[s.to].role === 'main';
        delta[s.from][0] += fx * (fromMain ? 0.28 : 1);
        delta[s.from][1] += fy * (fromMain ? 0.28 : 1);
        delta[s.to][0] -= fx * (toMain ? 0.28 : 1);
        delta[s.to][1] -= fy * (toMain ? 0.28 : 1);
      });

      for (var i = 0; i < nodes.length; i++) {
        for (var j = i + 1; j < nodes.length; j++) {
          var aNode = nodes[i];
          var bNode = nodes[j];
          var a2 = points[aNode.id];
          var b2 = points[bNode.id];
          if (!a2 || !b2) continue;
          var dx2 = b2[0] - a2[0];
          var dy2 = b2[1] - a2[1];
          var distance2 = Math.sqrt(dx2 * dx2 + dy2 * dy2) || 0.001;
          var minDistance = aNode.role === 'main' || bNode.role === 'main' ? 15 : 12;
          if (distance2 >= minDistance) continue;
          var force2 = (minDistance - distance2) * 0.18;
          var fx2 = (dx2 / distance2) * force2;
          var fy2 = (dy2 / distance2) * force2;
          delta[aNode.id][0] -= fx2 * (aNode.role === 'main' ? 0.25 : 1);
          delta[aNode.id][1] -= fy2 * (aNode.role === 'main' ? 0.25 : 1);
          delta[bNode.id][0] += fx2 * (bNode.role === 'main' ? 0.25 : 1);
          delta[bNode.id][1] += fy2 * (bNode.role === 'main' ? 0.25 : 1);
        }
      }

      Object.keys(adjacency).forEach(function (hubId) {
        var neighbors = adjacency[hubId];
        if (!neighbors || neighbors.length < 3) return;
        var center = points[hubId];
        if (!center) return;
        var info = [];
        for (var k = 0; k < neighbors.length; k++) {
          var p = points[neighbors[k]];
          if (!p) continue;
          info.push({ id: neighbors[k], angle: Math.atan2(p[1] - center[1], p[0] - center[0]) });
        }
        if (info.length < 3) return;
        info.sort(function (a, b) { return a.angle - b.angle; });
        var idealSep = (Math.PI * 2) / info.length;
        var threshold = idealSep * 0.85;
        for (var i2 = 0; i2 < info.length; i2++) {
          var aa = info[i2];
          var bb = info[(i2 + 1) % info.length];
          var diff = bb.angle - aa.angle;
          if (i2 === info.length - 1) diff += Math.PI * 2;
          if (diff >= threshold) continue;
          var fAng = (threshold - diff) * 0.7;
          var aTanX = -Math.sin(aa.angle);
          var aTanY = Math.cos(aa.angle);
          var bTanX = -Math.sin(bb.angle);
          var bTanY = Math.cos(bb.angle);
          var aMain = nodeById[aa.id] && nodeById[aa.id].role === 'main';
          var bMain = nodeById[bb.id] && nodeById[bb.id].role === 'main';
          var aScale = aMain ? 0.3 : 1;
          var bScale = bMain ? 0.3 : 1;
          if (delta[aa.id]) {
            delta[aa.id][0] -= aTanX * fAng * aScale;
            delta[aa.id][1] -= aTanY * fAng * aScale;
          }
          if (delta[bb.id]) {
            delta[bb.id][0] += bTanX * fAng * bScale;
            delta[bb.id][1] += bTanY * fAng * bScale;
          }
        }
      });

      var totalMovement = 0;
      nodes.forEach(function (n) {
        var p = points[n.id];
        var d = delta[n.id];
        if (!p || !d) return;
        var anchor = anchorById[n.id];
        if (anchor) {
          d[0] += (anchor[0] - p[0]) * 0.12;
          d[1] += (anchor[1] - p[1]) * 0.12;
        }
        totalMovement += Math.abs(d[0]) + Math.abs(d[1]);
        points[n.id] = [
          clampBoardPoint(p[0] + d[0], 5, 95),
          clampBoardPoint(p[1] + d[1], 6, 94)
        ];
      });
      // Early exit: once per-node movement averages below ~0.025 units there's
      // nothing visually changing — running the remaining iterations would
      // just burn CPU.
      if (step > 12 && totalMovement < nodes.length * 0.025) break;
    }
    return points;
  }

  function hierarchyPoints(nodes, layout, edges) {
    var points = {};
    var mains = nodes.filter(function (n) { return n.role === 'main'; });
    var mainAnchors = radialMainAnchors(mains.length, layout);
    mainAnchors.forEach(function (anchor, i) {
      if (mains[i]) points[mains[i].id] = anchor.point;
    });
    var branchesByParent = {};
    nodes.forEach(function (n) {
      if (n.role === 'main') return;
      var parentId = points[n.parentId] ? n.parentId : (mains[0] && mains[0].id);
      if (!parentId) return;
      if (!branchesByParent[parentId]) branchesByParent[parentId] = [];
      branchesByParent[parentId].push(n);
    });
    var crossDegree = {};
    nodes.forEach(function (n) { crossDegree[n.id] = 0; });
    var edgeList = edges || [];
    var nodeById = {};
    nodes.forEach(function (n) { nodeById[n.id] = n; });
    for (var ei = 0; ei < edgeList.length; ei++) {
      var e = edgeList[ei];
      if (!e || e.from === e.to) continue;
      var f = nodeById[e.from];
      var t = nodeById[e.to];
      if (!f || !t) continue;
      if (f.parentId === t.id || t.parentId === f.id) continue;
      crossDegree[e.from] = (crossDegree[e.from] || 0) + 1;
      crossDegree[e.to] = (crossDegree[e.to] || 0) + 1;
    }

    Object.keys(branchesByParent).forEach(function (parentId) {
      var mainIndex = mains.findIndex(function (n) { return n.id === parentId; });
      var anchor = mainAnchors[Math.max(0, mainIndex)] || { angle: -Math.PI / 2, point: [50, 46] };
      var branches = branchesByParent[parentId];
      var fullCircle = mains.length <= 1;
      var sector = fullCircle ? Math.PI * 2 : Math.min(2.15, (Math.PI * 2) / Math.max(2, mains.length * 0.95));
      branches.forEach(function (n, i) {
        var angle = fullCircle
          ? -Math.PI / 2 + (i * Math.PI * 2) / branches.length
          : anchor.angle - sector / 2 + (branches.length === 1 ? sector / 2 : (i * sector) / (branches.length - 1));
        var deg = crossDegree[n.id] || 0;
        var extraRing = deg === 0 ? 9 : (deg === 1 ? 4 : 0);
        var ringOffset = (fullCircle ? 0 : (i % 3) * 7) + extraRing;
        points[n.id] = [
          clampBoardPoint(50 + Math.cos(angle) * (44 + ringOffset), 6, 94),
          clampBoardPoint(50 + Math.sin(angle) * (38 + ringOffset), 7, 93)
        ];
      });
    });

    // Trivial graphs (no edges, or fewer than 3 nodes) don't benefit from
    // relaxation — the initial radial placement is already optimal.
    if (nodes.length < 3 || edgeList.length === 0) return points;

    rebalanceBranchesByBarycenter(branchesByParent, edgeList, points);
    var result = relaxPoints(nodes, edgeList, points, mainAnchors, mains, 50);
    rebalanceBranchesByBarycenter(branchesByParent, edgeList, result);
    result = relaxPoints(nodes, edgeList, result, mainAnchors, mains, 85);
    return result;
  }

  function compute(board, opts) {
    if (!board || typeof board !== 'object') return null;
    opts = opts || {};
    var maxNodes = opts.maxNodes != null ? opts.maxNodes : 18;
    var maxEdges = opts.maxEdges != null ? opts.maxEdges : 24;
    var fallbackTitle = opts.fallbackBoardTitle || 'Knowledge Board';
    var externalLabel = opts.externalNodeLabel || '外部';

    var rawNodes = (Array.isArray(board.nodes) ? board.nodes : [])
      .filter(function (n) { return n && typeof n.label === 'string' && n.label.trim(); })
      .slice(0, maxNodes);
    if (rawNodes.length < 2) return null;

    var hasExplicitHierarchy = rawNodes.some(function (n) {
      return (n.role && String(n.role).trim()) || (n.parent_id && String(n.parent_id).trim());
    });

    var drafts = rawNodes.map(function (n, i) {
      var externalSource = n.external_source ? String(n.external_source).trim() : '';
      return {
        id: (n.id && String(n.id)) || ('n' + (i + 1)),
        label: String(n.label).trim(),
        detail: (n.detail ? String(n.detail).trim() : ''),
        kind: normalizeKind(n.kind),
        role: hasExplicitHierarchy ? normalizeRole(n.role, n.kind, n.parent_id) : legacyRole(n.kind),
        parentId: (n.parent_id ? String(n.parent_id).trim() : ''),
        sourceType: normalizeSourceType(n.source_type, n.external_source),
        sourceLabel: externalSource || externalLabel
      };
    });

    var points;
    if (hasExplicitHierarchy) {
      if (!drafts.some(function (n) { return n.role === 'main'; })) {
        drafts[0].role = 'main';
      }
      var mainIds = {};
      drafts.forEach(function (n) { if (n.role === 'main') mainIds[n.id] = true; });
      var fallbackMain = null;
      for (var fi = 0; fi < drafts.length; fi++) {
        if (drafts[fi].role === 'main') { fallbackMain = drafts[fi]; break; }
      }
      var fallbackMainId = fallbackMain ? fallbackMain.id : drafts[0].id;
      drafts.forEach(function (n) {
        if (n.role === 'main') {
          n.parentId = '';
        } else if (!mainIds[n.parentId] || n.parentId === n.id) {
          n.parentId = fallbackMainId;
        }
      });
      points = hierarchyPoints(drafts, String(board.layout || 'grid').toLowerCase(), Array.isArray(board.edges) ? board.edges : []);
    } else {
      var fallbackPoints = whiteboardPoints(drafts.length, String(board.layout || 'grid').toLowerCase());
      points = {};
      drafts.forEach(function (n, i) { points[n.id] = fallbackPoints[i] || [50, 50]; });
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

    var rawEdges = Array.isArray(board.edges) ? board.edges.slice(0, maxEdges) : [];
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

    var edgeGeoms = validEdges.map(function (ve) {
      var from = ve.from, to = ve.to, i = ve.index;
      var trunk = (from.role === 'main' && to.role === 'main') || from.parentId === to.id || to.parentId === from.id;
      var redundant = !trunk && isRedundant(from.id, to.id);
      var dx = to.x - from.x;
      var dy = to.y - from.y;
      var len = Math.sqrt(dx * dx + dy * dy) || 1;
      var insetDist = Math.min(5.5, len * 0.18);
      var ux = dx / len, uy = dy / len;
      var ix1 = from.x + ux * insetDist;
      var iy1 = from.y + uy * insetDist;
      var ix2 = to.x - ux * insetDist;
      var iy2 = to.y - uy * insetDist;
      var nx = -dy / len, ny = dx / len;
      var curveSign = i % 2 === 0 ? 1 : -1;
      var curveMag = trunk ? 0.7 : (redundant ? 1.4 : 2.6);
      var midX = (ix1 + ix2) / 2;
      var midY = (iy1 + iy2) / 2;
      var cx = midX + nx * curveSign * curveMag;
      var cy = midY + ny * curveSign * curveMag;
      var anchorX = (ix1 + 2 * cx + ix2) / 4;
      var anchorY = (iy1 + 2 * cy + iy2) / 4;
      return { raw: ve.raw, index: i, from: from, to: to, trunk: trunk, redundant: redundant,
               ix1: ix1, iy1: iy1, ix2: ix2, iy2: iy2, cx: cx, cy: cy, anchorX: anchorX, anchorY: anchorY };
    });
    var allSegments = edgeGeoms.map(function (g) {
      return { x1: g.ix1, y1: g.iy1, x2: g.ix2, y2: g.iy2 };
    });
    var edges = edgeGeoms.map(function (geom, gi) {
      var label = geom.raw.label ? String(geom.raw.label).trim() : '';
      var labelWidth = clampBoardPoint(estimateLabelWidthEm(label), 5, 13.2);
      var labelHeight = 3.3;
      var otherSegs = [];
      for (var oi = 0; oi < allSegments.length; oi++) if (oi !== gi) otherSegs.push(allSegments[oi]);
      var lp = placeEdgeLabel(
        geom.anchorX, geom.anchorY,
        { x: geom.ix1, y: geom.iy1 }, { x: geom.ix2, y: geom.iy2 },
        labelWidth, labelHeight,
        occupiedLabelRects, nodeLabelRects,
        otherSegs,
        geom.index
      );
      return {
        id: geom.from.id + '-' + geom.to.id + '-' + geom.index,
        from: geom.from.id,
        to: geom.to.id,
        label: label,
        colorKind: edgeColorKind(geom.from, geom.to),
        colorSourceType: edgeColorSourceType(geom.from, geom.to),
        x1: geom.ix1, y1: geom.iy1,
        x2: geom.ix2, y2: geom.iy2,
        cx: geom.cx, cy: geom.cy,
        lx: lp.x, ly: lp.y,
        labelWidth: labelWidth,
        trunk: geom.trunk,
        redundant: geom.redundant
      };
    });

    return {
      title: (board.title ? String(board.title).trim() : '') || fallbackTitle,
      nodes: nodes,
      edges: edges
    };
  }

  global.WhiteboardLayout = { compute: compute };
})(typeof window !== 'undefined' ? window : globalThis);
