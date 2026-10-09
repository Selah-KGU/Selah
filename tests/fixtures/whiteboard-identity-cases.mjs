const node = (id, label, role = 'main', parent_id = '', node_type = 'structure') => ({
  id, label, detail: `本文 ${label}`, role, parent_id, node_type, kind: 'core', source_type: 'lecture',
});
export function duplicateBoard() {
  return { title: '旧白板の ID 衝突', normalized_by: 'backend', schema_version: 1, nodes: [
    node('a-3', '最初の主題'), node('a', '第二主題'),
    node('a-3', '衝突した分岐', 'branch', 'a-3'), node('term', '用語', 'branch', 'a-3', 'term'),
  ], edges: [{ from: 'a-3', to: 'a', label: '関係' }, { from: 'a-3', to: 'term', label: '' }] };
}
export function reservedBoard() {
  return { title: '予約済み ID', normalized_by: 'backend', nodes: [
    node('same', '主題 A'), node('same', '主題 B'), node('same-2', '既存後缀'), node('same-2-2', '既存後缀 2'),
  ], edges: [{ from: 'same', to: 'same-2', label: '既存参照' }] };
}
export function prototypeBoard() {
  return { title: '特殊名の主題', normalized_by: 'backend', nodes: [
    ...['__proto__', 'constructor', 'toString', 'hasOwnProperty'].map(id => node(id, `主題 ${id}`)),
    node('child', '分岐', 'branch', '__proto__'), node('term', '用語', 'branch', 'constructor', 'term'),
  ], edges: [
    { from: '__proto__', to: 'child', label: '分岐の関係' },
    { from: 'constructor', to: 'toString', label: '比較' },
    { from: 'hasOwnProperty', to: '__proto__', label: '関連' },
  ] };
}
export function phantomBoard() {
  return { title: '存在しない特殊名', normalized_by: 'backend', nodes: [
    node('root', '主題'), node('branch', '分岐', 'branch', 'constructor'),
    node('term', '孤立した用語', 'branch', '__proto__', 'term'),
  ], edges: [
    { from: '__proto__', to: 'root', label: '不存在' },
    { from: 'constructor', to: 'root', label: '不存在' },
    { from: 'root', to: 'branch', label: '実際の辺' },
  ] };
}
