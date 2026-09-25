import { Checkbox } from "./ui/checkbox";
import { NativeSelect } from "./ui/native-select";
import { Popover, PopoverContent, PopoverTrigger } from "./ui/popover";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "./ui/dropdown-menu";
import { ArrowRight, Bookmark, FilePlus, FolderPlus, ListX, Plus, Undo2, X } from "lucide-react";
import { Input } from "./ui/input";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

import { WindowControls } from "./WindowControls";
import { isImmersiveWindow } from "../lib/windowChrome";

type RuleType =
  | "textReplace"
  | "regexReplace"
  | "insertText"
  | "deleteRange"
  | "caseConversion"
  | "autoNumbering"
  | "stripBrackets";

type AnyRenameRule = {
  type: RuleType;
  id: string;
  find?: string;
  replacement?: string;
  pattern?: string;
  text?: string;
  position?: number | "prefix" | "suffix";
  from?: number;
  length?: number;
  mode?: "title" | "lower" | "upper";
  startAt?: number;
  padding?: number;
  separator?: string;
  bracketTypes?: Array<"square" | "round" | "curly">;
};

type FileEntry = {
  id: string;
  originalName: string;
  path: string;
};

type CompletedRename = { originalPath: string; newPath: string };

type RenamerOutcome = {
  renames: CompletedRename[];
  error: string | null;
  indexSyncFailures: number;
  skipped: number;
};

type PreviewResult = {
  id: string;
  originalName: string;
  newName: string;
  path: string;
  hasConflict: boolean;
  hasInvalidChars: boolean;
};

function newId() {
  return crypto.randomUUID();
}

function defaultRule(type: RuleType): AnyRenameRule {
  switch (type) {
    case "textReplace":
      return { type, id: newId(), find: "", replacement: "" };
    case "regexReplace":
      return { type, id: newId(), pattern: "", replacement: "" };
    case "insertText":
      return { type, id: newId(), text: "", position: 0 };
    case "deleteRange":
      return { type, id: newId(), from: 0, length: 1 };
    case "caseConversion":
      return { type, id: newId(), mode: "title" };
    case "autoNumbering":
      return {
        type,
        id: newId(),
        startAt: 1,
        padding: 2,
        position: "prefix",
        separator: " ",
      };
    case "stripBrackets":
      return { type, id: newId(), bracketTypes: ["square", "round"] };
  }
}

function isExecutable(p: PreviewResult) {
  return (
    !p.hasConflict &&
    !p.hasInvalidChars &&
    p.originalName !== p.newName &&
    p.newName.length > 0
  );
}

export function RenamerPage() {
  const { t } = useTranslation();
  const [files, setFiles] = useState<FileEntry[]>([]);
  const [rules, setRules] = useState<AnyRenameRule[]>([defaultRule("textReplace")]);
  const [previews, setPreviews] = useState<PreviewResult[]>([]);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [snapshotCount, setSnapshotCount] = useState(0);
  const [presets, setPresets] = useState<string[]>([]);
  const [selectedPreset, setSelectedPreset] = useState("");
  const [presetName, setPresetName] = useState("");
  const [rulesReady, setRulesReady] = useState(false);
  // Execution recomputes names from `files` + `rules` on the backend; only allow it
  // once the visible preview reflects exactly those inputs.
  const [previewCurrent, setPreviewCurrent] = useState(true);

  const executableCount = useMemo(
    () => previews.filter(isExecutable).length,
    [previews],
  );

  useEffect(() => {
    void (async () => {
      try {
        const [count, names, auto] = await Promise.all([
          invoke<number>("renamer_snapshot_count"),
          invoke<string[]>("renamer_list_presets"),
          invoke<{ rules: AnyRenameRule[] } | null>("renamer_auto_load_pipeline"),
        ]);
        setSnapshotCount(count);
        setPresets(names);
        if (auto?.rules?.length) {
          setRules(auto.rules);
        }
      } catch {
        setSnapshotCount(0);
      } finally {
        setRulesReady(true);
      }
    })();
  }, []);

  useEffect(() => {
    if (!rulesReady) return;
    const timer = window.setTimeout(() => {
      void invoke("renamer_auto_save_pipeline", { pipeline: { rules } }).catch(() => {});
    }, 400);
    return () => window.clearTimeout(timer);
  }, [rules, rulesReady]);

  useEffect(() => {
    let cancelled = false;
    setPreviewCurrent(false);
    const timer = window.setTimeout(() => {
      void (async () => {
        if (files.length === 0) {
          if (!cancelled) {
            setPreviews([]);
            setPreviewCurrent(true);
          }
          return;
        }
        try {
          const out = await invoke<PreviewResult[]>("renamer_preview", {
            files,
            pipeline: { rules },
          });
          if (!cancelled) {
            setPreviews(out);
            setPreviewCurrent(true);
          }
        } catch (err) {
          if (!cancelled) setMessage(String(err));
        }
      })();
    }, 120);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [files, rules]);

  async function addPaths(directory: boolean) {
    setMessage(null);
    const selected = await open({
      directory,
      multiple: !directory,
      title: directory ? t("renamer.pickFolder") : t("renamer.pickFiles"),
    });
    if (!selected) return;
    const paths = Array.isArray(selected) ? selected : [selected];
    try {
      setBusy(true);
      const collected = await invoke<FileEntry[]>("renamer_collect_files", { paths });
      setFiles((prev) => {
        const seen = new Set(prev.map((f) => f.path));
        const merged = [...prev];
        for (const f of collected) {
          if (!seen.has(f.path)) {
            seen.add(f.path);
            merged.push(f);
          }
        }
        return merged;
      });
    } catch (err) {
      setMessage(String(err));
    } finally {
      setBusy(false);
    }
  }

  async function applyOutcome(outcome: RenamerOutcome, doneKey: string) {
    const lines = [t(doneKey, { count: outcome.renames.length })];
    if (outcome.indexSyncFailures > 0) {
      lines.push(t("renamer.indexSyncFailed", { count: outcome.indexSyncFailures }));
    }
    if (outcome.skipped > 0) lines.push(t("renamer.undoSkipped", { count: outcome.skipped }));
    if (outcome.error) lines.push(outcome.error);
    setMessage(lines.join(" "));
    setSnapshotCount(await invoke<number>("renamer_snapshot_count"));
    if (outcome.renames.length === 0) return;
    // Follow moved entries so the list and preview keep pointing at real files.
    const moved = new Map(outcome.renames.map((r) => [r.originalPath, r.newPath]));
    if (!files.some((f) => moved.has(f.path))) return;
    const refreshed = await invoke<FileEntry[]>("renamer_collect_files", {
      paths: files.map((f) => moved.get(f.path) ?? f.path),
    });
    setFiles(refreshed);
  }

  async function runExecute() {
    setMessage(null);
    try {
      setBusy(true);
      const outcome = await invoke<RenamerOutcome>("renamer_execute", {
        files,
        pipeline: { rules },
      });
      await applyOutcome(outcome, "renamer.executed");
    } catch (err) {
      setMessage(String(err));
    } finally {
      setBusy(false);
    }
  }

  async function runUndo() {
    setMessage(null);
    try {
      setBusy(true);
      const outcome = await invoke<RenamerOutcome>("renamer_undo_last");
      await applyOutcome(outcome, "renamer.undone");
    } catch (err) {
      setMessage(String(err));
    } finally {
      setBusy(false);
    }
  }

  function updateRule(id: string, patch: Partial<AnyRenameRule>) {
    setRules((prev) => prev.map((r) => (r.id === id ? { ...r, ...patch } : r)));
  }

  async function refreshPresets() {
    const names = await invoke<string[]>("renamer_list_presets");
    setPresets(names);
  }

  async function loadPreset(name: string) {
    if (!name) return;
    setMessage(null);
    try {
      const pipeline = await invoke<{ rules: AnyRenameRule[] } | null>("renamer_load_preset", {
        name,
      });
      if (pipeline?.rules) {
        setRules(pipeline.rules);
        setSelectedPreset(name);
        setMessage(t("renamer.preset.loaded", { name }));
      }
    } catch (err) {
      setMessage(String(err));
    }
  }

  async function savePreset() {
    const name = (presetName.trim() || selectedPreset).trim();
    if (!name) {
      setMessage(t("renamer.preset.nameRequired"));
      return;
    }
    setMessage(null);
    try {
      await invoke("renamer_save_preset", { name, pipeline: { rules } });
      await refreshPresets();
      setSelectedPreset(name);
      setPresetName("");
      setMessage(t("renamer.preset.saved", { name }));
    } catch (err) {
      setMessage(String(err));
    }
  }

  async function deletePreset() {
    const name = selectedPreset;
    if (!name) return;
    setMessage(null);
    try {
      await invoke("renamer_delete_preset", { name });
      await refreshPresets();
      setSelectedPreset("");
      setMessage(t("renamer.preset.deleted", { name }));
    } catch (err) {
      setMessage(String(err));
    }
  }

  const immersive = isImmersiveWindow();
  const rows: PreviewRow[] = useMemo(() => (previews.length > 0
    ? previews.map((p) => ({ ...p, status: previewStatus(p), runs: diffRuns(p.originalName, p.newName) }))
    : files.map((f) => ({ id: f.id, originalName: f.originalName, newName: f.originalName, path: f.path, hasConflict: false, hasInvalidChars: false, status: "same" as const, runs: [{ text: f.originalName, changed: false }] }))
  ), [previews, files]);
  const conflicts = rows.filter((r) => r.status === "clash" || r.status === "bad").length;
  const unchanged = rows.filter((r) => r.status === "same").length;
  const folder = commonFolder(files.map((f) => f.path));

  return (
    <div className="sl-rn">
      {immersive ? <div className="absolute right-0 top-0 z-30 h-[var(--kg-titlebar-height)]"><WindowControls /></div> : null}
      <header className="sl-toolbar sl-rn-tool">
        <div data-tauri-drag-region />
        <div className="sl-title"><b>{t("renamer.title")}</b>{folder ? <span title={folder}>{folder}</span> : null}</div>
        <button type="button" className="sl-btn" disabled={busy} onClick={() => void addPaths(false)}><FilePlus aria-hidden />{t("renamer.addFiles")}</button>
        <button type="button" className="sl-btn" disabled={busy} onClick={() => void addPaths(true)}><FolderPlus aria-hidden />{t("renamer.addFolder")}</button>
        {files.length > 0 ? (
          <button type="button" className="sl-iconbtn" aria-label={t("renamer.clearFiles")} title={t("renamer.clearFiles")} onClick={() => setFiles([])}><ListX aria-hidden /></button>
        ) : null}
        <Popover>
          <PopoverTrigger asChild>
            <button type="button" className="sl-iconbtn" aria-label={t("renamer.presets")} title={t("renamer.presets")}><Bookmark aria-hidden /></button>
          </PopoverTrigger>
          <PopoverContent align="end" sideOffset={6} className="sl-rn-presets">
            <p className="kg-section-label">{t("renamer.presets")}</p>
            <div className="flex gap-2">
              <NativeSelect className="min-w-0 flex-1" value={selectedPreset} onChange={(e) => void loadPreset(e.target.value)}>
                <option value="">{t("renamer.preset.pick")}</option>
                {presets.map((name) => <option key={name} value={name}>{name}</option>)}
              </NativeSelect>
              <button type="button" className="sl-btn" disabled={!selectedPreset} onClick={() => void deletePreset()}>{t("common.remove")}</button>
            </div>
            <div className="flex gap-2">
              <Input className="min-w-0 flex-1" placeholder={t("renamer.preset.namePlaceholder")} value={presetName} onChange={(e) => setPresetName(e.target.value)} />
              <button type="button" className="sl-btn" onClick={() => void savePreset()}>{t("renamer.preset.save")}</button>
            </div>
          </PopoverContent>
        </Popover>
      </header>

      <div className="sl-rn-body">
        <aside className="sl-rn-rules">
          <div className="sl-rn-rh">
            <span>{t("renamer.rulesOrdered")}</span>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <button type="button" className="sl-btn"><Plus aria-hidden />{t("renamer.addRule")}</button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" sideOffset={6}>
                {RULE_TYPES.map((type) => (
                  <DropdownMenuItem key={type} onSelect={() => setRules((prev) => [...prev, defaultRule(type)])}>{t(`renamer.rule.${type}`)}</DropdownMenuItem>
                ))}
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
          {rules.length === 0 ? <p className="sl-rn-empty">{t("renamer.rulesEmpty")}</p> : null}
          {rules.map((rule, index) => (
            <RuleEditor key={rule.id} index={index} rule={rule}
              onChange={(patch) => updateRule(rule.id, patch)}
              onRemove={() => setRules((prev) => prev.filter((r) => r.id !== rule.id))} />
          ))}
        </aside>

        <main className="sl-rn-prev">
          {message ? <p className="sl-rn-msg" role="status">{message}</p> : null}
          {rows.length === 0 ? (
            <div className="sl-rn-empty grow">{t("renamer.filesEmpty")}</div>
          ) : (
            <div className="sl-rn-table" role="table" aria-label={t("renamer.preview")}>
              <div className="sl-rr head" role="row">
                <span>{t("renamer.col.original")}</span><span /><span>{t("renamer.col.new")}</span><span>{t("renamer.col.status")}</span>
              </div>
              {rows.map((r, i) => {
                return (
                  <div key={r.id} className="sl-rr" role="row" data-odd={i % 2 === 1 || undefined}>
                    <span className="o" title={r.path}>{r.originalName}</span>
                    <ArrowRight className="arr" aria-hidden />
                    <span className="nw" title={r.newName}>
                      {r.runs.map((run, k) => (run.changed ? <mark key={k}>{run.text}</mark> : run.text))}
                    </span>
                    <span className="st" data-status={r.status}><i aria-hidden />{t(STATUS_LABEL[r.status])}</span>
                  </div>
                );
              })}
            </div>
          )}
          <footer className="sl-rn-foot">
            <span>
              {t("renamer.summary", { total: rows.length, count: executableCount })}
              {conflicts > 0 ? <> · <span className="warn">{t("renamer.summaryConflicts", { count: conflicts })}</span></> : null}
              {unchanged > 0 ? <> · {t("renamer.summaryUnchanged", { count: unchanged })}</> : null}
            </span>
            <span className="grow" />
            <button type="button" className="sl-btn" disabled={busy || snapshotCount === 0} onClick={() => void runUndo()}>
              <Undo2 aria-hidden />{t("renamer.undoLast")}
            </button>
            <button type="button" className="sl-btn acc" disabled={busy || !previewCurrent || executableCount === 0} onClick={() => void runExecute()}>
              {t("renamer.executeN", { count: executableCount })}
            </button>
          </footer>
        </main>
      </div>
    </div>
  );
}

type RowStatus = "go" | "same" | "clash" | "bad";
type PreviewRow = PreviewResult & { status: RowStatus; runs: { text: string; changed: boolean }[] };

const RULE_TYPES: RuleType[] = ["textReplace", "regexReplace", "insertText", "deleteRange", "caseConversion", "autoNumbering", "stripBrackets"];

const STATUS_LABEL: Record<RowStatus, string> = {
  go: "renamer.status.ok",
  same: "renamer.status.unchanged",
  clash: "renamer.status.conflict",
  bad: "renamer.status.invalid",
};

function previewStatus(p: PreviewResult): RowStatus {
  if (p.hasConflict) return "clash";
  if (p.hasInvalidChars) return "bad";
  return p.originalName === p.newName ? "same" : "go";
}

/** Runs of `next`, flagged where they are not carried over from `prev` (character LCS). */
function diffRuns(prev: string, next: string): { text: string; changed: boolean }[] {
  if (prev === next) return [{ text: next, changed: false }];
  const a = Array.from(prev);
  const b = Array.from(next);
  // Long names would make the table quadratic; mark the whole name instead.
  if (a.length * b.length > 90_000) return [{ text: next, changed: true }];
  const w = b.length + 1;
  const lcs = new Uint16Array((a.length + 1) * w);
  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      lcs[i * w + j] = a[i] === b[j] ? lcs[(i + 1) * w + j + 1] + 1 : Math.max(lcs[(i + 1) * w + j], lcs[i * w + j + 1]);
    }
  }
  const runs: { text: string; changed: boolean }[] = [];
  const push = (ch: string, changed: boolean) => {
    const last = runs[runs.length - 1];
    if (last && last.changed === changed) last.text += ch;
    else runs.push({ text: ch, changed });
  };
  let i = 0;
  let j = 0;
  while (j < b.length) {
    if (i < a.length && a[i] === b[j]) { push(b[j], false); i++; j++; }
    else if (i < a.length && lcs[(i + 1) * w + j] >= lcs[i * w + j + 1]) i++;
    else { push(b[j], true); j++; }
  }
  return runs;
}

function commonFolder(paths: string[]): string {
  if (paths.length === 0) return "";
  const split = paths.map((p) => p.split(/[/\\]/).slice(0, -1));
  const first = split[0];
  let n = first.length;
  for (const parts of split.slice(1)) {
    let i = 0;
    while (i < n && i < parts.length && parts[i] === first[i]) i++;
    n = i;
  }
  const sep = paths[0].includes("\\") && !paths[0].includes("/") ? "\\" : "/";
  return first.slice(0, n).join(sep);
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return <label className="sl-rf"><span>{label}</span>{children}</label>;
}

function RuleEditor({
  index,
  rule,
  onChange,
  onRemove,
}: {
  index: number;
  rule: AnyRenameRule;
  onChange: (patch: Partial<AnyRenameRule>) => void;
  onRemove: () => void;
}) {
  const { t } = useTranslation();
  const num = (value: string) => Number(value) || 0;
  return (
    <div className="sl-rule">
      <div className="sl-rule-top">
        <span className="n">{index + 1}</span>
        <b>{t(`renamer.rule.${rule.type}`)}</b>
        <button type="button" className="x" aria-label={t("common.remove")} title={t("common.remove")} onClick={onRemove}><X aria-hidden /></button>
      </div>
      {(rule.type === "textReplace" || rule.type === "regexReplace") && (
        <div className="sl-rfs">
          <Field label={rule.type === "regexReplace" ? t("renamer.field.pattern") : t("renamer.field.find")}>
            <Input value={rule.type === "regexReplace" ? (rule.pattern ?? "") : (rule.find ?? "")}
              onChange={(e) => onChange(rule.type === "regexReplace" ? { pattern: e.target.value } : { find: e.target.value })} />
          </Field>
          <Field label={t("renamer.field.replacement")}>
            <Input placeholder={t("renamer.field.deletePlaceholder")} value={rule.replacement ?? ""} onChange={(e) => onChange({ replacement: e.target.value })} />
          </Field>
        </div>
      )}
      {rule.type === "insertText" && (
        <div className="sl-rfs">
          <Field label={t("renamer.field.text")}>
            <Input value={rule.text ?? ""} onChange={(e) => onChange({ text: e.target.value })} />
          </Field>
          <Field label={t("renamer.field.position")}>
            <Input className="w-20" type="number" value={typeof rule.position === "number" ? rule.position : 0} onChange={(e) => onChange({ position: num(e.target.value) })} />
          </Field>
        </div>
      )}
      {rule.type === "deleteRange" && (
        <div className="sl-rfs">
          <Field label={t("renamer.field.from")}>
            <Input className="w-20" type="number" value={rule.from ?? 0} onChange={(e) => onChange({ from: num(e.target.value) })} />
          </Field>
          <Field label={t("renamer.field.length")}>
            <Input className="w-20" type="number" value={rule.length ?? 1} onChange={(e) => onChange({ length: num(e.target.value) })} />
          </Field>
        </div>
      )}
      {rule.type === "caseConversion" && (
        <div className="sl-seg" role="radiogroup" aria-label={t("renamer.rule.caseConversion")}>
          {(["title", "lower", "upper"] as const).map((mode) => (
            <button key={mode} type="button" role="radio" aria-checked={(rule.mode ?? "title") === mode} onClick={() => onChange({ mode })}>{t(`renamer.case.${mode}`)}</button>
          ))}
        </div>
      )}
      {rule.type === "autoNumbering" && (
        <div className="sl-rfs">
          <Field label={t("renamer.field.startAt")}>
            <Input className="w-20" type="number" value={rule.startAt ?? 1} onChange={(e) => onChange({ startAt: num(e.target.value) })} />
          </Field>
          <Field label={t("renamer.field.padding")}>
            <Input className="w-20" type="number" value={rule.padding ?? 2} onChange={(e) => onChange({ padding: num(e.target.value) })} />
          </Field>
          <Field label={t("renamer.field.separator")}>
            <Input className="w-20" value={rule.separator ?? " "} onChange={(e) => onChange({ separator: e.target.value })} />
          </Field>
          <Field label={t("renamer.field.position")}>
            <div className="sl-seg" role="radiogroup" aria-label={t("renamer.field.position")}>
              {(["prefix", "suffix"] as const).map((pos) => (
                <button key={pos} type="button" role="radio" aria-checked={(rule.position === "suffix" ? "suffix" : "prefix") === pos} onClick={() => onChange({ position: pos })}>{t(`renamer.pos.${pos}`)}</button>
              ))}
            </div>
          </Field>
        </div>
      )}
      {rule.type === "stripBrackets" && (
        <div className="flex flex-wrap gap-3 text-[12px] text-fg-secondary">
          {(["square", "round", "curly"] as const).map((b) => {
            const checked = (rule.bracketTypes ?? []).includes(b);
            return (
              <label key={b} className="inline-flex items-center gap-1.5">
                <Checkbox checked={checked} onCheckedChange={(e) => {
                  const cur = new Set(rule.bracketTypes ?? []);
                  if (e === true) cur.add(b);
                  else cur.delete(b);
                  onChange({ bracketTypes: Array.from(cur) });
                }} />
                {t(`renamer.bracket.${b}`)}
              </label>
            );
          })}
        </div>
      )}
    </div>
  );
}
