export function boardFixture({ count = 24, edges = 48, labelled = 1, hierarchy = "backend", seed = 1 } = {}) {
  let state = seed >>> 0;
  const random = max => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return state % max;
  };
  const mains = Math.min(count, 3);
  const nodes = Array.from({ length: count }, (_, i) => {
    const main = i < mains;
    const term = !main && i % 7 === 0;
    const parent = main ? "" : `n${random(i)}`;
    return {
      id: `n${i}`, label: `テーマ ${i} / 中文・日本語 🙂`,
      detail: "詳細 ".repeat(random(30)),
      node_type: term ? "term" : "structure",
      kind: ["core", "support", "result", "question"][random(4)],
      ...(hierarchy === "legacy" ? {} : { role: main ? "main" : "branch", parent_id: parent }),
      source_type: i % 4 === 0 ? "external" : "lecture",
      external_source: i % 4 === 0 ? "https://example.invalid/source" : "",
    };
  });
  const structureIds = nodes.filter(n => n.node_type !== "term").map(n => n.id);
  // Terms can attach to structures, as in native normalized boards.
  for (const node of nodes) {
    if (node.parent_id && nodes.find(n => n.id === node.parent_id)?.node_type === "term") node.parent_id = "n0";
  }
  const connections = Array.from({ length: edges }, (_, i) => {
    const from = random(structureIds.length);
    const to = (from + 1 + random(structureIds.length - 1)) % structureIds.length;
    return {
      from: structureIds[from], to: structureIds[to],
      label: i / edges < labelled ? ["関連", " 比較 / A → B ", "長いラベル・条件・関係 🙂"][i % 3] : "",
    };
  });
  return {
    title: " 知識整理 | A,B ", layout: ["grid", "flow", "compare", "cycle", "hub"][seed % 5],
    ...(hierarchy === "backend" ? { normalized_by: "backend", schema_version: 1 } : {}),
    nodes, edges: connections,
  };
}

export function layoutCases() {
  const cases = [null, undefined, {}, { nodes: [] }, { nodes: [{ label: "one" }] },
    { nodes: [null, { label: " " }, { label: 4 }, { label: "one" }] }];
  for (const hierarchy of ["backend", "explicit", "legacy"]) {
    for (const count of [2, 3, 6, 12, 24, 75]) {
      for (const labelled of [0, 0.25, 1]) {
        for (const seed of [1, 2, 17]) cases.push(boardFixture({ hierarchy, count, edges: count * 2, labelled, seed }));
      }
    }
  }
  const hierarchy = boardFixture({ count: 18, edges: 40 });
  cases.push({ ...hierarchy, title: "", nodes: hierarchy.nodes.map(n => ({ ...n, id: n.id.replace("n", "a,b|") })), edges: [] });
  cases.push({ ...hierarchy, edges: [null, {}, { from: "n0", to: "n0", label: "self" },
    { from: "missing", to: "n1" }, { from: "n0", to: "n1", label: 7 },
    { from: "n1", to: "n0", label: "  " }, ...hierarchy.edges] });
  cases.push({ ...hierarchy, nodes: hierarchy.nodes.map(n => ({ ...n, role: "branch", parent_id: "missing" })) });
  cases.push({ ...hierarchy, nodes: hierarchy.nodes.map(n => ({ ...n, role: "branch", parent_id: n.id })) });
  cases.push({ ...hierarchy, nodes: hierarchy.nodes.map((n, i) => ({ ...n, role: "branch", parent_id: `n${(i + 1) % hierarchy.nodes.length}` })) });
  cases.push({ ...hierarchy, nodes: hierarchy.nodes.map(n => ({ ...n, node_type: "term", parent_id: "missing" })) });
  cases.push({ ...hierarchy, nodes: hierarchy.nodes.map(n => ({ ...n, label: "  x  ", detail: "" })), edges: [] });
  for (const layout of ["grid", "flow", "compare", "cycle", "hub", "unknown"]) {
    cases.push({ ...boardFixture({ hierarchy: "legacy", count: 8 }), layout });
  }
  return cases;
}
