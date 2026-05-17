import type * as Monaco from "monaco-editor";

// Minimal SAS tokenizer for v0.1. Real grammar lands later.
export function registerSasLanguage(monaco: typeof Monaco) {
  if (monaco.languages.getLanguages().some((l) => l.id === "sas")) return;

  monaco.languages.register({ id: "sas" });

  monaco.languages.setMonarchTokensProvider("sas", {
    ignoreCase: true,
    keywords: [
      "data", "run", "proc", "quit", "set", "merge", "by", "where",
      "if", "then", "else", "do", "end", "select", "when", "otherwise",
      "output", "delete", "stop", "return", "retain", "keep", "drop",
      "rename", "length", "format", "informat", "label", "array",
      "libname", "filename", "options", "title", "footnote",
      "create", "table", "as", "from", "join", "left", "right", "inner",
      "outer", "on", "union", "all", "corr", "group", "order", "having",
      "calculated", "distinct", "into",
    ],
    operators: ["=", "<", ">", "<=", ">=", "ne", "lt", "le", "gt", "ge", "and", "or", "not", "||"],
    tokenizer: {
      root: [
        [/\/\*/, "comment", "@blockComment"],
        [/^\s*\*[^;]*;/, "comment"],
        [/'[^']*'[dt]?/, "string"],
        [/"[^"]*"[dt]?/, "string"],
        [/&[a-z_][a-z0-9_]*\.?/i, "variable"],
        [/%[a-z_][a-z0-9_]*/i, "annotation"],
        [/[a-z_][a-z0-9_]*/i, {
          cases: {
            "@keywords": "keyword",
            "@default": "identifier",
          },
        }],
        [/\d+\.?\d*/, "number"],
        [/[;,]/, "delimiter"],
      ],
      blockComment: [
        [/[^*/]+/, "comment"],
        [/\*\//, "comment", "@pop"],
        [/[*/]/, "comment"],
      ],
    },
  });

  monaco.languages.setLanguageConfiguration("sas", {
    comments: { lineComment: "*", blockComment: ["/*", "*/"] },
    brackets: [
      ["(", ")"],
      ["[", "]"],
      ["{", "}"],
    ],
    autoClosingPairs: [
      { open: "(", close: ")" },
      { open: "[", close: "]" },
      { open: "{", close: "}" },
      { open: '"', close: '"' },
      { open: "'", close: "'" },
    ],
  });
}
