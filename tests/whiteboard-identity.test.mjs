import test from 'node:test';
import assert from 'node:assert/strict';
import { loadWhiteboardLayout } from './load-whiteboard-layout.mjs';
import { duplicateBoard, reservedBoard, prototypeBoard, phantomBoard } from './fixtures/whiteboard-identity-cases.mjs';
const layout = await loadWhiteboardLayout();
const clone = value => structuredClone(value);
const ids = result => result.nodes.map(node => node.id);
const finite = result => {
  for (const node of result.nodes) assert.ok(Number.isFinite(node.x) && Number.isFinite(node.y), node.id);
  for (const edge of result.edges) for (const key of ['x1','y1','x2','y2','cx','cy','lx','ly']) assert.ok(Number.isFinite(edge[key]), `${edge.id}/${key}`);
};

test('old duplicate IDs retain every structure and term with unique renderer keys and original references', () => {
  const board = duplicateBoard(), original = clone(board);
  const result = layout.compute(board);
  assert.deepEqual(ids(result), ['a-3', 'a', 'a-3-3']);
  assert.equal(new Set(ids(result)).size, 3);
  assert.deepEqual(result.nodes.map(node => node.label), ['最初の主題', '第二主題', '衝突した分岐']);
  assert.equal(result.nodes[2].parentId, 'a-3');
  assert.equal(result.nodes[0].chips[0].label, '用語');
  assert.deepEqual(result.edges.map(edge => [edge.from, edge.to]), [['a-3','a']]);
  finite(result);
  assert.deepEqual(board, original);
});

test('topic enumeration and filtered layouts reserve future IDs and agree on renamed topics', () => {
  const board = reservedBoard();
  const expected = ['same', 'same-2-3', 'same-2', 'same-2-2'];
  assert.deepEqual(layout.topics(board).map(topic => topic.id), expected);
  assert.deepEqual(ids(layout.compute(board)), expected);
  for (const topic of layout.topics(board)) {
    const result = layout.compute(board, { topicIds: [topic.id] });
    assert.deepEqual(ids(result), [topic.id]);
    assert.equal(result.nodes[0].label, topic.label);
    finite(result);
  }
});

test('missing and zero IDs do not consume a later explicit ID or its reserved suffix', () => {
  const board = reservedBoard();
  board.nodes = [{ label:'fallback',role:'main' },
    ...['n1','n1-1','n1-1-2'].map(id => ({id,label:id,role:'main'})), {id:0,label:'zero',role:'main'}];
  board.edges = [];
  const expected = ['n1-1-3', 'n1', 'n1-1', 'n1-1-2', 'n5'];
  assert.deepEqual(ids(layout.compute(board)), expected);
  assert.deepEqual(layout.topics(board).map(topic => topic.id), expected);
});

test('prototype-property names are ordinary IDs in trees, terms, topics and edges', () => {
  const board = prototypeBoard(), original = clone(board);
  const result = layout.compute(board);
  assert.deepEqual(ids(result), ['__proto__','constructor','toString','hasOwnProperty','child']);
  assert.equal(result.nodes[1].chips[0].label, '用語');
  assert.equal(result.edges.length, 3);
  finite(result);
  for (const topic of layout.topics(board)) {
    const selected = layout.compute(board, { topicIds: [topic.id] });
    assert.ok(ids(selected).includes(topic.id));
    assert.ok(selected.nodes.every(node => node.id === topic.id || node.parentId === topic.id));
    finite(selected);
  }
  assert.deepEqual(board, original);
});

test('missing prototype names never become phantom endpoints or mutate built-in objects', () => {
  const descriptor = Object.getOwnPropertyDescriptor(Object.prototype, 'chips');
  try {
    const board = phantomBoard();
    const result = layout.compute(board);
    assert.deepEqual(Object.getOwnPropertyDescriptor(Object.prototype, 'chips'), descriptor);
    assert.deepEqual(result.edges.map(edge => edge.label), ['実際の辺']);
    assert.equal(result.nodes[0].chips[0].label, '孤立した用語');
    finite(result);
  } finally {
    if (descriptor) Object.defineProperty(Object.prototype, 'chips', descriptor);
    else delete Object.prototype.chips;
  }
});

test('legacy and explicit boards use the same identity repair without rewriting input data', () => {
  for (const hierarchy of ['explicit','legacy']) {
    for (const factory of [duplicateBoard, reservedBoard, prototypeBoard]) {
      const board = factory(); delete board.normalized_by;
      if (hierarchy === 'legacy') for (const node of board.nodes) { delete node.role; delete node.parent_id; }
      const original = clone(board);
      const result = layout.compute(board);
      assert.equal(new Set(ids(result)).size, result.nodes.length);
      finite(result);
      assert.deepEqual(board, original);
      for (const topic of layout.topics(board)) assert.ok(ids(result).includes(topic.id));
    }
  }
});
