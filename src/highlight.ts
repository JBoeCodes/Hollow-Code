// Zero-dependency syntax highlighter tuned for the languages that actually
// show up in Hollow chat (TS/JS, Rust, Python, JSON, shell, CSS, HTML, diff).
//
// Safety: tokenize() returns an array of {type, value} tokens; the renderer
// (components/Highlight.tsx) emits each as a Solid text node or a <span> with
// a text child — both auto-escaped. No innerHTML, so nothing here can inject
// markup even on adversarial input.
//
// Design: a sticky-regex scanner. At each position we try an ordered rule
// list; the first rule that matches at the cursor wins and advances it. Every
// rule's RegExp carries the `y` (sticky) flag so a match can only start at the
// cursor — that's what keeps the scan linear and position-correct. Unmatched
// characters fall through as a single 'plain' char; adjacent same-type tokens
// are merged so the DOM stays light.

export type TokType =
  | 'keyword' | 'string' | 'comment' | 'number' | 'function' | 'type'
  | 'builtin' | 'operator' | 'punct' | 'property' | 'tag' | 'attr'
  | 'meta' | 'inserted' | 'deleted' | 'plain';

export interface Tok { t: TokType; v: string; }
interface Rule { t: TokType; re: RegExp; }

const WS: Rule = { t: 'plain', re: /\s+/y };

// ── JavaScript / TypeScript (and JSX/TSX) ──
const jsRules: Rule[] = [
  WS,
  { t: 'comment',  re: /\/\/[^\n]*/y },
  { t: 'comment',  re: /\/\*[\s\S]*?\*\//y },
  { t: 'string',   re: /`(?:\\[\s\S]|[^\\`])*`/y },
  { t: 'string',   re: /"(?:\\[\s\S]|[^\\"\n])*"/y },
  { t: 'string',   re: /'(?:\\[\s\S]|[^\\'\n])*'/y },
  { t: 'number',   re: /\b0[xX][0-9a-fA-F]+\b|\b0[bB][01]+\b|\b\d[\d_]*(?:\.\d+)?(?:[eE][+-]?\d+)?n?\b/y },
  { t: 'keyword',  re: /\b(?:abstract|as|asserts|async|await|break|case|catch|class|const|continue|debugger|declare|default|delete|do|else|enum|export|extends|finally|for|from|function|get|if|implements|import|in|infer|instanceof|interface|is|keyof|let|namespace|new|of|private|protected|public|readonly|return|satisfies|set|static|super|switch|this|throw|try|type|typeof|var|void|while|yield)\b/y },
  { t: 'builtin',  re: /\b(?:true|false|null|undefined|NaN|Infinity)\b/y },
  { t: 'builtin',  re: /\b(?:string|number|boolean|any|unknown|never|object|symbol|bigint)\b/y },
  { t: 'function', re: /[A-Za-z_$][\w$]*(?=\s*\()/y },
  { t: 'type',     re: /\b[A-Z][\w$]*\b/y },
  { t: 'operator', re: /=>|\.\.\.|\?\?|&&|\|\||[+\-*/%=<>!&|^~?]=?/y },
  { t: 'punct',    re: /[{}()\[\];,.:@]/y },
  { t: 'plain',    re: /[A-Za-z_$][\w$]*/y },
];

// ── Rust ──
const rustRules: Rule[] = [
  WS,
  { t: 'comment',  re: /\/\/[^\n]*/y },
  { t: 'comment',  re: /\/\*[\s\S]*?\*\//y },
  { t: 'string',   re: /b?"(?:\\[\s\S]|[^\\"])*"/y },
  { t: 'string',   re: /'(?:\\.|[^\\'])'/y },
  { t: 'meta',     re: /#!?\[[^\]]*\]/y },
  { t: 'number',   re: /\b0[xX][0-9a-fA-F_]+|\b\d[\d_]*(?:\.\d+)?(?:[eE][+-]?\d+)?(?:[iuf](?:8|16|32|64|128|size))?\b/y },
  { t: 'keyword',  re: /\b(?:as|async|await|break|const|continue|crate|dyn|else|enum|extern|fn|for|if|impl|in|let|loop|match|mod|move|mut|pub|ref|return|self|static|struct|super|trait|type|unsafe|use|where|while)\b/y },
  { t: 'builtin',  re: /\b(?:Self|true|false|Some|None|Ok|Err|Option|Result|Vec|String|Box|Rc|Arc|HashMap|i8|i16|i32|i64|i128|isize|u8|u16|u32|u64|u128|usize|f32|f64|bool|char|str)\b/y },
  { t: 'type',     re: /'[a-z_]\w*/y },
  { t: 'function', re: /\b[a-z_]\w*!/y },
  { t: 'function', re: /[A-Za-z_]\w*(?=\s*[(<])/y },
  { t: 'type',     re: /\b[A-Z]\w*\b/y },
  { t: 'operator', re: /->|=>|::|\.\.=?|[+\-*/%=<>!&|^~?]=?/y },
  { t: 'punct',    re: /[{}()\[\];,.:@#]/y },
  { t: 'plain',    re: /[A-Za-z_]\w*/y },
];

// ── Python ──
const pyRules: Rule[] = [
  WS,
  { t: 'comment',  re: /#[^\n]*/y },
  { t: 'string',   re: /[rbfu]{0,2}"""[\s\S]*?"""/iy },
  { t: 'string',   re: /[rbfu]{0,2}'''[\s\S]*?'''/iy },
  { t: 'string',   re: /[rbfu]{0,2}"(?:\\[\s\S]|[^\\"\n])*"/iy },
  { t: 'string',   re: /[rbfu]{0,2}'(?:\\[\s\S]|[^\\'\n])*'/iy },
  { t: 'meta',     re: /@[A-Za-z_]\w*/y },
  { t: 'number',   re: /\b0[xX][0-9a-fA-F]+|\b\d[\d_]*(?:\.\d+)?(?:[eE][+-]?\d+)?j?\b/y },
  { t: 'keyword',  re: /\b(?:and|as|assert|async|await|break|class|continue|def|del|elif|else|except|finally|for|from|global|if|import|in|is|lambda|nonlocal|not|or|pass|raise|return|try|while|with|yield|match|case)\b/y },
  { t: 'builtin',  re: /\b(?:True|False|None|self|cls|print|len|range|int|str|float|list|dict|set|tuple|bool|super|isinstance|enumerate|zip|map|filter|open|Exception)\b/y },
  { t: 'function', re: /[A-Za-z_]\w*(?=\s*\()/y },
  { t: 'type',     re: /\b[A-Z]\w*\b/y },
  { t: 'operator', re: /\*\*|\/\/|->|:=|[+\-*/%=<>!&|^~]=?/y },
  { t: 'punct',    re: /[{}()\[\];,.:@]/y },
  { t: 'plain',    re: /[A-Za-z_]\w*/y },
];

// ── JSON ──
const jsonRules: Rule[] = [
  WS,
  { t: 'property', re: /"(?:\\[\s\S]|[^\\"])*"(?=\s*:)/y },
  { t: 'string',   re: /"(?:\\[\s\S]|[^\\"])*"/y },
  { t: 'number',   re: /-?\b\d[\d_]*(?:\.\d+)?(?:[eE][+-]?\d+)?\b/y },
  { t: 'builtin',  re: /\b(?:true|false|null)\b/y },
  { t: 'punct',    re: /[{}\[\],:]/y },
];

// ── Shell / Bash ──
const bashRules: Rule[] = [
  WS,
  { t: 'comment',  re: /#[^\n]*/y },
  { t: 'string',   re: /"(?:\\[\s\S]|[^\\"])*"/y },
  { t: 'string',   re: /'[^']*'/y },
  { t: 'property', re: /\$\{[^}]*\}|\$[A-Za-z_]\w*|\$[0-9@*#?]/y },
  { t: 'keyword',  re: /\b(?:if|then|else|elif|fi|for|while|until|do|done|case|esac|in|function|select|return|break|continue|local|export|declare|readonly|source|alias|set|unset|trap)\b/y },
  { t: 'builtin',  re: /\b(?:echo|cd|ls|cat|grep|sed|awk|cp|mv|rm|mkdir|touch|chmod|chown|curl|wget|git|npm|npx|cargo|node|python|pip|sudo|kill|ps|find|xargs|tar|ssh|exit|printf|read|test)\b/y },
  { t: 'number',   re: /\b\d+\b/y },
  { t: 'operator', re: /&&|\|\||[|&;]|<<?|>>?/y },
  { t: 'punct',    re: /[{}()\[\];,]/y },
  { t: 'plain',    re: /[A-Za-z_][\w-]*/y },
];

// ── CSS / SCSS ──
const cssRules: Rule[] = [
  WS,
  { t: 'comment',  re: /\/\*[\s\S]*?\*\//y },
  { t: 'string',   re: /"(?:\\[\s\S]|[^\\"])*"|'(?:\\[\s\S]|[^\\'])*'/y },
  { t: 'keyword',  re: /@[A-Za-z-]+/y },
  { t: 'number',   re: /#[0-9a-fA-F]{3,8}\b/y },
  { t: 'number',   re: /-?\b\d*\.?\d+(?:px|em|rem|%|vh|vw|s|ms|fr|deg|pt|ex|ch)?\b/y },
  { t: 'builtin',  re: /--[A-Za-z-]+/y },
  { t: 'property', re: /[A-Za-z-]+(?=\s*:)/y },
  { t: 'function', re: /[A-Za-z-]+(?=\()/y },
  { t: 'type',     re: /\.[A-Za-z_][\w-]*|#[A-Za-z_][\w-]*|::?[a-z-]+|&/y },
  { t: 'punct',    re: /[{}();,]/y },
  { t: 'plain',    re: /[A-Za-z_][\w-]*/y },
];

// ── HTML / XML ──
const htmlRules: Rule[] = [
  WS,
  { t: 'comment',  re: /<!--[\s\S]*?-->/y },
  { t: 'meta',     re: /<!doctype[^>]*>/iy },
  { t: 'tag',      re: /<\/?[A-Za-z][\w-]*/y },
  { t: 'punct',    re: /\/?>/y },
  { t: 'string',   re: /"[^"]*"|'[^']*'/y },
  { t: 'attr',     re: /[A-Za-z_:][\w:.-]*(?==)/y },
  { t: 'operator', re: /=/y },
  { t: 'plain',    re: /[^<\s="']+/y },
];

function run(code: string, rules: Rule[]): Tok[] {
  const out: Tok[] = [];
  let i = 0;
  const n = code.length;
  while (i < n) {
    let matched = false;
    for (const r of rules) {
      r.re.lastIndex = i;
      const m = r.re.exec(code);
      if (m && m[0].length > 0) {
        out.push({ t: r.t, v: m[0] });
        i += m[0].length;
        matched = true;
        break;
      }
    }
    if (!matched) {
      out.push({ t: 'plain', v: code[i] });
      i++;
    }
  }
  return merge(out);
}

function merge(toks: Tok[]): Tok[] {
  const out: Tok[] = [];
  for (const tk of toks) {
    const last = out[out.length - 1];
    if (last && last.t === tk.t) last.v += tk.v;
    else out.push({ t: tk.t, v: tk.v });
  }
  return out;
}

// Diff is line-oriented, not token-oriented: classify each whole line so the
// renderer can tint inserted/deleted rows full-width.
function diffTokens(code: string): Tok[] {
  const out: Tok[] = [];
  const lines = code.split('\n');
  lines.forEach((line, idx) => {
    let t: TokType = 'plain';
    if (/^(?:\+\+\+|---|diff |index |@@)/.test(line)) t = 'meta';
    else if (line.startsWith('+')) t = 'inserted';
    else if (line.startsWith('-')) t = 'deleted';
    out.push({ t, v: line + (idx < lines.length - 1 ? '\n' : '') });
  });
  return out;
}

function grammar(lang: string): Rule[] | 'diff' | null {
  switch (lang) {
    case 'ts': case 'tsx': case 'typescript': case 'js': case 'jsx':
    case 'javascript': case 'mjs': case 'cjs': case 'json5':
      return jsRules;
    case 'rust': case 'rs': return rustRules;
    case 'py': case 'python': return pyRules;
    case 'json': case 'jsonc': return jsonRules;
    case 'sh': case 'bash': case 'shell': case 'zsh': case 'console':
      return bashRules;
    case 'css': case 'scss': case 'less': return cssRules;
    case 'html': case 'xml': case 'svg': case 'vue': return htmlRules;
    case 'diff': case 'patch': return 'diff';
    default: return null;
  }
}

/** Tokenize `code` for `lang`. Unknown languages return a single plain token
 *  (still rendered, just uncolored). Never throws. */
export function tokenize(code: string, lang?: string): Tok[] {
  const g = grammar((lang || '').toLowerCase());
  if (g === 'diff') return diffTokens(code);
  if (!g) return [{ t: 'plain', v: code ?? '' }];
  try {
    return run(code ?? '', g);
  } catch {
    return [{ t: 'plain', v: code ?? '' }];
  }
}

