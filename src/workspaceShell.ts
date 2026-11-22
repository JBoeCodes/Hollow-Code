// Which output files are bookkeeping rather than Results: small, so the
// App chunk can read it.

/** A saved canvas's record (`outputs/canvases/<id>.canvas.json`) or anything in a dot folder
 *  (`.revisions/`): store bookkeeping, not a result. A canvas state save
 *  rewrites only the record, so counting it read as a changed result (C3).
 *  Files a curated section canvas left (`_plan.canvas.html` /
 *  `.canvas.json`, C6 · K1; the view was removed by COO rebuild L4) were a
 *  panel view, not a deliverable. Archive older entries' files
 *  (`outputs/workspace-archive/`) are moved brief and plan text, which the
 *  Rust results listing leaves out too (overnight 10 A10). */
export const isResultBookkeeping = (name: string) =>
  /^outputs\/canvases\/[^/]+\.canvas\.json$/.test(name)
  || /^outputs\/workspace-archive\//.test(name)
  || /^outputs\/canvases\/_(overview|brief|plan|sources)\.canvas\.(html|json)$/.test(name)
  || /(^|\/)\./.test(name);
