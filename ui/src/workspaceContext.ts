import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import type { ColumnInfo, DatasetInfo, Library, LogLine, TabConfig } from "./types";

/**
 * Periodically (on every engine refresh) snapshots libraries, datasets, and
 * column schemas into a plain-text summary for the Agent's system context.
 */
export function useSchemaContext(refreshToken: number): string {
  const [schemaContext, setSchemaContext] = useState<string>("");

  useEffect(() => {
    let active = true;
    const fetchSchema = async () => {
      try {
        const libs = await invoke<Library[]>("list_libraries");
        const schemas: string[] = [];
        for (const lib of libs) {
          if (!active) return;
          try {
            const datasets = await invoke<DatasetInfo[]>("list_datasets", { libref: lib.name });
            if (datasets.length === 0) {
              schemas.push(`- Library: ${lib.name.toUpperCase()} (${lib.kind})${lib.path ? ` at ${lib.path}` : ""} (empty)`);
              continue;
            }
            schemas.push(`- Library: ${lib.name.toUpperCase()} (${lib.kind})${lib.path ? ` at ${lib.path}` : ""}`);
            for (const ds of datasets) {
              if (!active) return;
              try {
                const cols = await invoke<ColumnInfo[]>("dataset_schema", { libref: lib.name, name: ds.name });
                const colStr = cols.map(c => `      - ${c.name}: ${c.ty}`).join("\n");
                schemas.push(`  - Dataset: ${ds.name} (${ds.rows ?? 0} rows)\n${colStr}`);
              } catch (e) {
                schemas.push(`  - Dataset: ${ds.name} (${ds.rows ?? 0} rows) (Error reading columns: ${String(e)})`);
              }
            }
          } catch (e) {
            schemas.push(`- Library: ${lib.name.toUpperCase()} (${lib.kind}) (Error listing datasets: ${String(e)})`);
          }
        }
        if (active) {
          setSchemaContext(schemas.join("\n"));
        }
      } catch (e) {
        console.error("Failed to build schema context", e);
      }
    };
    fetchSchema();
    return () => {
      active = false;
    };
  }, [refreshToken]);

  return schemaContext;
}

function escapeXml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

export interface WorkspaceContextInput {
  activeTab: { path: string | null } | null;
  tabs: { path: string | null; content: string }[];
  projectName: string | null;
  projectPrograms: TabConfig[];
  schemaContext: string;
  log: LogLine[];
}

/**
 * Builds the XML-ish workspace snapshot handed to the Agent: active file,
 * project files (with content), database schemas, and recent diagnostics.
 */
export function buildWorkspaceContext({
  activeTab,
  tabs,
  projectName,
  projectPrograms,
  schemaContext,
  log,
}: WorkspaceContextInput): string {
  const xmlParts: string[] = [];

  // 1. Active file path
  if (activeTab && activeTab.path) {
    xmlParts.push(`  <active_file path="${activeTab.path}" />`);
  } else {
    xmlParts.push('  <active_file path="untitled.pas" />');
  }

  // 2. Project structure and file contents
  if (projectName) {
    xmlParts.push(`  <active_project name="${projectName}">`);
    if (projectPrograms.length > 0) {
      const programXml = projectPrograms.map(p => {
        const openTab = tabs.find(t => t.path === p.path);
        const code = openTab ? openTab.content : (p.content || "");
        // Escape standard XML chars to ensure robust structural parsing by the LLM
        return `    <file path="${p.path}">\n${escapeXml(code)}\n    </file>`;
      }).join("\n");
      xmlParts.push(programXml);
    }
    xmlParts.push("  </active_project>");
  }

  // 3. Database schemas
  if (schemaContext) {
    xmlParts.push("  <database_schema>");
    xmlParts.push(schemaContext.split("\n").map(line => `    ${line}`).join("\n"));
    xmlParts.push("  </database_schema>");
  }

  // 4. Execution Diagnostics
  const diagnostics = log.filter(line => line.level === "error" || line.level === "warning");
  if (diagnostics.length > 0) {
    const recentDiag = diagnostics.slice(-10);
    xmlParts.push("  <execution_diagnostics>");
    recentDiag.forEach(d => {
      // Escape content inside diagnostics to prevent malformed XML tags
      xmlParts.push(`    <diagnostic level="${d.level}">${escapeXml(d.text)}</diagnostic>`);
    });
    xmlParts.push("  </execution_diagnostics>");
  }

  return xmlParts.join("\n");
}
